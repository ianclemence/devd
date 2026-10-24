//! Declaring a capability whose arguments are a typed struct.
//!
//! ```ignore
//! registry
//!     .define::<PinArgs>("site.set_php", "Pin a site to a PHP version")
//!     .description("Pins a site to one of the installed PHP versions ...")
//!     .group("site")
//!     .tier(Tier::Write)
//!     .effect(Effect::Config)
//!     .agent_safe()
//!     .idempotent()
//!     .handlers(|ctx, args| plan(ctx, args), |ctx, args| apply(ctx, args))?;
//! ```
//!
//! Every property is named. A capability used to be declared by one function
//! with a dozen positional arguments, three of them adjacent booleans, copied into
//! each crate; swapping `agent_safe` and `idempotent` compiled and silently
//! changed who may call it. Here a flag that is not named is false, and there is
//! one implementation of how the typed arguments become the registry's untyped
//! handler.

use std::marker::PhantomData;
use std::sync::Arc;

use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::Value;

use crate::capability::{Capability, CapabilityBuilder, Ctx, Effect, Outcome, Plan, Tier};
use crate::error::{DevdError, Result};
use crate::registry::Registry;

/// A capability being declared. Finish it with [`Define::handlers`].
#[must_use = "a capability is registered by calling .handlers(...)"]
pub struct Define<'r, A> {
    registry: &'r mut Registry,
    id: String,
    builder: CapabilityBuilder,
    _args: PhantomData<fn() -> A>,
}

impl Registry {
    /// Begin declaring a capability whose arguments deserialize into `A`. The
    /// JSON schema the interfaces and agents read is derived from `A`.
    pub fn define<A>(&mut self, id: &str, summary: &str) -> Define<'_, A> {
        Define {
            registry: self,
            id: id.to_string(),
            // The builder defaults to agent_safe; here nothing is allowed that is not named.
            builder: Capability::builder(id, summary).agent_safe(false),
            _args: PhantomData,
        }
    }
}

impl<A> Define<'_, A> {
    /// The longer description, which says what it does, to whom and with what
    /// consequence.
    pub fn description(mut self, text: impl Into<String>) -> Self {
        self.builder = self.builder.description(text);
        self
    }

    /// The discovery group.
    pub fn group(mut self, group: impl Into<String>) -> Self {
        self.builder = self.builder.group(group);
        self
    }

    /// The permission tier.
    pub fn tier(mut self, tier: Tier) -> Self {
        self.builder = self.builder.tier(tier);
        self
    }

    /// What it changes.
    pub fn effect(mut self, effect: Effect) -> Self {
        self.builder = self.builder.effect(effect);
        self
    }

    /// An agent may call it.
    pub fn agent_safe(mut self) -> Self {
        self.builder = self.builder.agent_safe(true);
        self
    }

    /// Only a person may call it.
    pub fn human_only(mut self) -> Self {
        self.builder = self.builder.human_only(true);
        self
    }

    /// An agent's call waits for a person to approve it, because the caller supplies the code
    /// that will run. A person calling it is not asked.
    pub fn agent_needs_approval(mut self) -> Self {
        self.builder = self.builder.agent_needs_approval(true);
        self
    }

    /// It can describe what it would do without doing it.
    pub fn dry_run(mut self) -> Self {
        self.builder = self.builder.dry_run(true);
        self
    }

    /// Calling it twice has the effect of calling it once.
    pub fn idempotent(mut self) -> Self {
        self.builder = self.builder.idempotent(true);
        self
    }

    /// Supply what produces its plan and what applies it, and register it.
    pub fn handlers<P, Q>(self, plan: P, apply: Q) -> Result<()>
    where
        A: for<'de> Deserialize<'de> + JsonSchema + Send + Sync + 'static,
        P: Fn(&Ctx, &A) -> Result<Plan> + Send + Sync + 'static,
        Q: Fn(&Ctx, &A) -> Result<Outcome> + Send + Sync + 'static,
    {
        let schema = serde_json::to_value(schemars::schema_for!(A))
            .map_err(|e| DevdError::internal(format!("cannot build the schema for {}: {e}", self.id)))?;
        let capability = self
            .builder
            .args_schema(schema)
            .handler(
                move |ctx: &Ctx, args: &Value| plan(ctx, &ctx.parse::<A>(args)?),
                move |ctx: &Ctx, args: &Value| apply(ctx, &ctx.parse::<A>(args)?),
            )
            .build()?;
        self.registry.register(capability)
    }
}

impl<A> Define<'_, A> {
    /// Like [`Define::handlers`], for a capability that runs against a service
    /// shared with other capabilities: both handlers receive it.
    pub fn handlers_with<S, P, Q>(self, service: &Arc<S>, plan: P, apply: Q) -> Result<()>
    where
        A: for<'de> Deserialize<'de> + JsonSchema + Send + Sync + 'static,
        S: Send + Sync + 'static,
        P: Fn(&Ctx, &A, &S) -> Result<Plan> + Send + Sync + 'static,
        Q: Fn(&Ctx, &A, &S) -> Result<Outcome> + Send + Sync + 'static,
    {
        let for_plan = Arc::clone(service);
        let for_apply = Arc::clone(service);
        self.handlers(
            move |ctx, args| plan(ctx, args, &for_plan),
            move |ctx, args| apply(ctx, args, &for_apply),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    use crate::capability::{Actor, Surface};
    use crate::config::Settings;
    use crate::events::EventBus;
    use crate::paths::Paths;
    use crate::state::Store;

    fn test_ctx() -> Ctx {
        let store = Store::open_in_memory().unwrap();
        store.migrate().unwrap();
        Ctx {
            store: Arc::new(store),
            paths: Arc::new(Paths::rooted("C:/tmp/devd-test")),
            settings: Arc::new(Settings::default()),
            events: EventBus::new(),
            actor: Actor::human(Surface::Cli),
            dry_run: false,
        }
    }

    #[derive(Debug, Deserialize, JsonSchema)]
    struct Echo {
        text: String,
    }

    #[test]
    fn a_capability_is_what_its_named_properties_say() {
        let mut registry = Registry::new();
        registry
            .define::<Echo>("demo.echo", "Say it back")
            .description("Returns the text it was given.")
            .group("demo")
            .tier(Tier::Read)
            .effect(Effect::Read)
            .agent_safe()
            .idempotent()
            .handlers(
                |_, a| Ok(Plan::new("demo.echo", format!("echo {}", a.text))),
                |_, a| Ok(Outcome::with_data("echoed", json!({ "text": a.text }))),
            )
            .unwrap();

        let cap = registry.get("demo.echo").expect("registered");
        assert_eq!(cap.group, "demo");
        assert_eq!(cap.tier, Tier::Read);
        assert!(cap.agent_safe && cap.idempotent);
        // What was not named is false: nobody asked for these.
        assert!(!cap.human_only && !cap.supports_dry_run);
        assert_eq!(cap.args_schema["required"], json!(["text"]));
    }

    #[test]
    fn flags_that_are_not_named_stay_off() {
        let mut registry = Registry::new();
        registry
            .define::<Echo>("demo.quiet", "Does nothing special")
            .group("demo")
            .tier(Tier::Write)
            .effect(Effect::Write)
            .handlers(|_, _| Ok(Plan::new("demo.quiet", "x")), |_, _| Ok(Outcome::ok("x")))
            .unwrap();
        let cap = registry.get("demo.quiet").unwrap();
        assert!(!cap.agent_safe && !cap.human_only && !cap.idempotent && !cap.supports_dry_run);
    }

    #[test]
    fn arguments_that_do_not_fit_the_type_are_refused_before_the_handler_runs() {
        let mut registry = Registry::new();
        registry
            .define::<Echo>("demo.strict", "Needs text")
            .group("demo")
            .tier(Tier::Read)
            .effect(Effect::Read)
            .handlers(
                |_, a| Ok(Plan::new("demo.strict", a.text.clone())),
                |_, _| panic!("the handler must not run with bad arguments"),
            )
            .unwrap();
        let cap = registry.get("demo.strict").unwrap();
        let ctx = test_ctx();
        assert!(cap.handler.plan(&ctx, &json!({ "text": 3 })).is_err());
        assert!(cap.handler.plan(&ctx, &json!({ "text": "hi" })).is_ok());
    }
}
