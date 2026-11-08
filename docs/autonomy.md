# Working on its own: the rules

This is the checklist a change must pass if it lets devd act without being asked,
or lets an agent record something instead of doing it. It exists because the
features that make devd feel like a partner — proposals, rules, heartbeats,
queues — are also the features that can quietly spend a person's money or trust.

1. **Nothing starts work that a person has not approved.** A proposal is a row.
   A rule in `observe` mode records; in `auto` it starts a run, and that run still
   waits for plan approval. There is no mode in which a rule, a suggestion or an
   agent starts work that skips the plan approval.
2. **The event is a fact devd recorded, never a model's opinion.** An intake rule
   is judged against a health check's own state, a run's own terminal state, a
   schedule's own last error. If a model is needed to recognise the event, it is
   not an intake trigger.
3. **A proposal is recorded once.** The same title for the same project while it
   is still open is one row. A rule that fires every minute does not fill the
   Inbox, and an agent cannot flood the person with the same idea.
4. **Every run of devd's own work is visible afterwards.** A proposal, a rule's
   match, a queue row, a stall and a revoked mandate are all rows with a time. If
   a person cannot read what happened and when, the feature is not done.
5. **The retries are bounded and the failures are named.** A queue row backs off
   and is dropped after about a week with the reason. A rule that no longer
   applies starts its count again rather than firing late.
6. **A healthy system is quiet.** A health entry reports an error only after three
   failures in a row, and clears on the next success. A run with nothing waiting
   asks for nothing.
7. **The words come from one place.** A run's question, the sentence a person
   reads and the actions that resolve it are produced by one function
   (`devd_agent::needs`), not by each surface. Two screens cannot disagree.
8. **The heartbeat measures activity, not liveness.** A worker waiting on a slow
   model is not stalled; the stall window is longer than the provider's own
   request timeout, and stopping a worker is `unverified` with a reason, never
   `failed` without one.
9. **Nothing here is a permission.** A mandate explains why devd believed it could
   run a task; the registry's tiers, the approval service and the evidence rules
   are still what decide. A good trust record never removes a person's decision.

When adding a capability that does any of this, put its test next to the existing
ones in `crates/devd-agent/tests/autonomy.rs` or `crates/devd-ops/src/effects.rs`,
and add a line to [`agents.md`](agents.md) saying what a person now has to decide.
