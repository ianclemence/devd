//! Persistence for AI providers, the model catalogue, built-in agent
//! conversations, the inbox and agent approval requests.

use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::state::Store;
use crate::util;

/// A configured model provider. The API key is never part of this row.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    /// Identifier, e.g. `anthropic`.
    pub id: String,
    /// `anthropic`, `openai` or `openai-compatible`.
    pub kind: String,
    /// Display name.
    pub label: String,
    /// API base URL.
    pub base_url: String,
    /// Environment variable that holds the key, when the key is not stored.
    pub key_env: Option<String>,
    /// Model used when a thread names none.
    pub default_model: Option<String>,
    /// The provider new conversations use.
    pub is_default: bool,
    /// Usable.
    pub enabled: bool,
    /// Created.
    pub created_at: String,
    /// Updated.
    pub updated_at: String,
}

/// A model in the catalogue.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CatalogueModel {
    /// Where it came from: `anthropic-docs` or `provider:<id>`.
    pub source: String,
    /// The id to send to the API.
    pub model_id: String,
    /// Display name.
    pub display: String,
    /// Description.
    pub description: Option<String>,
    /// Context window in tokens.
    pub context_window: Option<i64>,
    /// Maximum output tokens.
    pub max_output: Option<i64>,
    /// USD per million input tokens.
    pub input_price: Option<f64>,
    /// USD per million output tokens.
    pub output_price: Option<f64>,
    /// Retirement note.
    pub retirement: Option<String>,
    /// Page the data came from.
    pub url: Option<String>,
    /// When it was fetched.
    pub fetched_at: String,
}

/// A built-in agent conversation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatThread {
    /// Identifier.
    pub id: String,
    /// Title.
    pub title: String,
    /// Project directory the agent works in.
    pub project: Option<String>,
    /// Provider id.
    pub provider_id: Option<String>,
    /// Model.
    pub model: Option<String>,
    /// `idle`, `running`, `waiting_approval` or `error`.
    pub state: String,
    /// Last error.
    pub error: Option<String>,
    /// Input tokens used.
    pub input_tokens: i64,
    /// Output tokens used.
    pub output_tokens: i64,
    /// Created.
    pub created_at: String,
    /// Updated.
    pub updated_at: String,
}

/// One entry in a conversation's log.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChatEvent {
    /// Sequence number.
    pub id: i64,
    /// Thread.
    pub thread_id: String,
    /// `user`, `assistant`, `tool_result`, `notice` or `error`.
    pub kind: String,
    /// JSON payload.
    pub data: serde_json::Value,
    /// When.
    pub created_at: String,
}

/// A message to the person, from an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboxMessage {
    /// Identifier.
    pub id: String,
    /// Who sent it.
    pub from_actor: String,
    /// Project it concerns.
    pub project: Option<String>,
    /// Subject.
    pub subject: String,
    /// Body.
    pub body: String,
    /// The agent is waiting for an answer.
    pub needs_reply: bool,
    /// `open` or `resolved`.
    pub state: String,
    /// The person's reply.
    pub reply: Option<String>,
    /// When replied.
    pub replied_at: Option<String>,
    /// Created.
    pub created_at: String,
}

/// An operation an agent asked a person to approve.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRequest {
    /// Identifier.
    pub id: String,
    /// The agent, e.g. `agent:claude-code`.
    pub actor: String,
    /// Capability.
    pub capability: String,
    /// Arguments.
    pub args: serde_json::Value,
    /// The plan the capability produced.
    pub plan: Option<serde_json::Value>,
    /// `pending`, `approved`, `declined` or `failed`.
    pub state: String,
    /// What the operation returned.
    pub result: Option<serde_json::Value>,
    /// Why it failed.
    pub error: Option<String>,
    /// Conversation it belongs to.
    pub thread_id: Option<String>,
    /// The tool call waiting for it.
    pub tool_use_id: Option<String>,
    /// Created.
    pub created_at: String,
    /// Decided.
    pub decided_at: Option<String>,
    /// Who decided.
    pub decided_by: Option<String>,
}

fn j(v: &Option<String>) -> Option<serde_json::Value> {
    v.as_ref().and_then(|s| serde_json::from_str(s).ok())
}

fn row_provider(r: &rusqlite::Row) -> rusqlite::Result<Provider> {
    Ok(Provider {
        id: r.get(0)?,
        kind: r.get(1)?,
        label: r.get(2)?,
        base_url: r.get(3)?,
        key_env: r.get(4)?,
        default_model: r.get(5)?,
        is_default: r.get::<_, i32>(6)? != 0,
        enabled: r.get::<_, i32>(7)? != 0,
        created_at: r.get(8)?,
        updated_at: r.get(9)?,
    })
}

const PROVIDER_COLS: &str = "id,kind,label,base_url,key_env,default_model,is_default,enabled,created_at,updated_at";
const THREAD_COLS: &str = "id,title,project,provider_id,model,state,error,input_tokens,output_tokens,created_at,updated_at";
const REQ_COLS: &str = "id,actor,capability,args,plan,state,result,error,thread_id,tool_use_id,created_at,decided_at,decided_by";

fn row_thread(r: &rusqlite::Row) -> rusqlite::Result<ChatThread> {
    Ok(ChatThread {
        id: r.get(0)?,
        title: r.get(1)?,
        project: r.get(2)?,
        provider_id: r.get(3)?,
        model: r.get(4)?,
        state: r.get(5)?,
        error: r.get(6)?,
        input_tokens: r.get(7)?,
        output_tokens: r.get(8)?,
        created_at: r.get(9)?,
        updated_at: r.get(10)?,
    })
}

fn row_request(r: &rusqlite::Row) -> rusqlite::Result<AgentRequest> {
    let args: String = r.get(3)?;
    let plan: Option<String> = r.get(4)?;
    let result: Option<String> = r.get(6)?;
    Ok(AgentRequest {
        id: r.get(0)?,
        actor: r.get(1)?,
        capability: r.get(2)?,
        args: serde_json::from_str(&args).unwrap_or(serde_json::Value::Null),
        plan: j(&plan),
        state: r.get(5)?,
        result: j(&result),
        error: r.get(7)?,
        thread_id: r.get(8)?,
        tool_use_id: r.get(9)?,
        created_at: r.get(10)?,
        decided_at: r.get(11)?,
        decided_by: r.get(12)?,
    })
}

impl Store {
    // ------------------------------------------------------------ providers

    /// Insert or update a provider (never touches its key).
    pub fn upsert_provider(&self, p: &Provider) -> Result<()> {
        self.with(|c| {
            if p.is_default {
                c.execute("UPDATE providers SET is_default=0 WHERE id<>?1", params![p.id])?;
            }
            c.execute(
                "INSERT INTO providers (id,kind,label,base_url,key_env,default_model,is_default,enabled,created_at,updated_at)
                 VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10)
                 ON CONFLICT(id) DO UPDATE SET kind=?2,label=?3,base_url=?4,key_env=?5,default_model=?6,is_default=?7,enabled=?8,updated_at=?10",
                params![p.id, p.kind, p.label, p.base_url, p.key_env, p.default_model, p.is_default as i32, p.enabled as i32, p.created_at, p.updated_at],
            )?;
            Ok(())
        })
    }

    /// Every provider.
    pub fn list_providers(&self) -> Result<Vec<Provider>> {
        self.with(|c| {
            let mut st = c.prepare(&format!("SELECT {PROVIDER_COLS} FROM providers ORDER BY is_default DESC, label"))?;
            let rows = st.query_map([], row_provider)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// One provider.
    pub fn get_provider(&self, id: &str) -> Result<Option<Provider>> {
        self.with(|c| Ok(c.query_row(&format!("SELECT {PROVIDER_COLS} FROM providers WHERE id=?1"), params![id], row_provider).optional()?))
    }

    /// Remove a provider and its cached models.
    pub fn delete_provider(&self, id: &str) -> Result<bool> {
        self.with(|c| {
            c.execute("DELETE FROM model_catalogue WHERE source=?1", params![format!("provider:{id}")])?;
            Ok(c.execute("DELETE FROM providers WHERE id=?1", params![id])? > 0)
        })
    }

    /// Make one provider the default.
    pub fn set_default_provider(&self, id: &str) -> Result<()> {
        self.with(|c| {
            c.execute("UPDATE providers SET is_default=(id=?1)", params![id])?;
            Ok(())
        })
    }

    // ------------------------------------------------------------ catalogue

    /// Replace the catalogue entries from one source.
    pub fn replace_catalogue(&self, source: &str, models: &[CatalogueModel]) -> Result<()> {
        self.with(|c| {
            let tx = c.unchecked_transaction()?;
            tx.execute("DELETE FROM model_catalogue WHERE source=?1", params![source])?;
            for m in models {
                tx.execute(
                    "INSERT OR REPLACE INTO model_catalogue (source,model_id,display,description,context_window,max_output,input_price,output_price,retirement,url,fetched_at)
                     VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)",
                    params![source, m.model_id, m.display, m.description, m.context_window, m.max_output, m.input_price, m.output_price, m.retirement, m.url, m.fetched_at],
                )?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Catalogue entries, optionally for one source.
    pub fn list_catalogue(&self, source: Option<&str>) -> Result<Vec<CatalogueModel>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT source,model_id,display,description,context_window,max_output,input_price,output_price,retirement,url,fetched_at
                 FROM model_catalogue WHERE (?1 IS NULL OR source=?1) ORDER BY source, rowid",
            )?;
            let rows = st.query_map(params![source], |r| {
                Ok(CatalogueModel {
                    source: r.get(0)?,
                    model_id: r.get(1)?,
                    display: r.get(2)?,
                    description: r.get(3)?,
                    context_window: r.get(4)?,
                    max_output: r.get(5)?,
                    input_price: r.get(6)?,
                    output_price: r.get(7)?,
                    retirement: r.get(8)?,
                    url: r.get(9)?,
                    fetched_at: r.get(10)?,
                })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    // -------------------------------------------------------------- threads

    /// Create a thread.
    pub fn create_thread(&self, title: &str, project: Option<&str>, provider_id: Option<&str>, model: Option<&str>) -> Result<ChatThread> {
        let now = util::now();
        let t = ChatThread {
            id: util::new_id(),
            title: title.to_string(),
            project: project.map(|s| s.to_string()),
            provider_id: provider_id.map(|s| s.to_string()),
            model: model.map(|s| s.to_string()),
            state: "idle".into(),
            error: None,
            input_tokens: 0,
            output_tokens: 0,
            created_at: now.clone(),
            updated_at: now,
        };
        self.with(|c| {
            c.execute(
                &format!("INSERT INTO chat_threads ({THREAD_COLS}) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11)"),
                params![t.id, t.title, t.project, t.provider_id, t.model, t.state, t.error, t.input_tokens, t.output_tokens, t.created_at, t.updated_at],
            )?;
            Ok(())
        })?;
        Ok(t)
    }

    /// One thread.
    pub fn get_thread(&self, id: &str) -> Result<Option<ChatThread>> {
        self.with(|c| Ok(c.query_row(&format!("SELECT {THREAD_COLS} FROM chat_threads WHERE id=?1"), params![id], row_thread).optional()?))
    }

    /// Recent threads.
    pub fn list_threads(&self, limit: i64) -> Result<Vec<ChatThread>> {
        self.with(|c| {
            let mut st = c.prepare(&format!("SELECT {THREAD_COLS} FROM chat_threads ORDER BY updated_at DESC LIMIT ?1"))?;
            let rows = st.query_map(params![limit], row_thread)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Update a thread's state.
    pub fn set_thread_state(&self, id: &str, state: &str, error: Option<&str>) -> Result<()> {
        self.with(|c| {
            c.execute("UPDATE chat_threads SET state=?2, error=?3, updated_at=?4 WHERE id=?1", params![id, state, error, util::now()])?;
            Ok(())
        })
    }

    /// Update a thread's title or model selection.
    pub fn update_thread(&self, id: &str, title: Option<&str>, provider_id: Option<&str>, model: Option<&str>) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE chat_threads SET title=COALESCE(?2,title), provider_id=COALESCE(?3,provider_id), model=COALESCE(?4,model), updated_at=?5 WHERE id=?1",
                params![id, title, provider_id, model, util::now()],
            )?;
            Ok(())
        })
    }

    /// Add to a thread's token counters.
    pub fn add_thread_usage(&self, id: &str, input: i64, output: i64) -> Result<()> {
        self.with(|c| {
            c.execute("UPDATE chat_threads SET input_tokens=input_tokens+?2, output_tokens=output_tokens+?3 WHERE id=?1", params![id, input, output])?;
            Ok(())
        })
    }

    /// Delete a thread and its log.
    pub fn delete_thread(&self, id: &str) -> Result<bool> {
        self.with(|c| {
            c.execute("DELETE FROM chat_events WHERE thread_id=?1", params![id])?;
            Ok(c.execute("DELETE FROM chat_threads WHERE id=?1", params![id])? > 0)
        })
    }

    /// Append to a thread's log. Returns the sequence number.
    pub fn append_event(&self, thread_id: &str, kind: &str, data: &serde_json::Value) -> Result<i64> {
        self.with(|c| {
            c.execute(
                "INSERT INTO chat_events (thread_id,kind,data,created_at) VALUES (?1,?2,?3,?4)",
                params![thread_id, kind, data.to_string(), util::now()],
            )?;
            c.execute("UPDATE chat_threads SET updated_at=?2 WHERE id=?1", params![thread_id, util::now()])?;
            Ok(c.last_insert_rowid())
        })
    }

    /// Events after `after` (exclusive), oldest first.
    pub fn list_events(&self, thread_id: &str, after: i64, limit: i64) -> Result<Vec<ChatEvent>> {
        self.with(|c| {
            let mut st = c.prepare("SELECT id,thread_id,kind,data,created_at FROM chat_events WHERE thread_id=?1 AND id>?2 ORDER BY id LIMIT ?3")?;
            let rows = st.query_map(params![thread_id, after, limit], |r| {
                let data: String = r.get(3)?;
                Ok(ChatEvent { id: r.get(0)?, thread_id: r.get(1)?, kind: r.get(2)?, data: serde_json::from_str(&data).unwrap_or(serde_json::Value::Null), created_at: r.get(4)? })
            })?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    // ---------------------------------------------------------------- inbox

    /// Post a message.
    pub fn post_inbox(&self, from: &str, project: Option<&str>, subject: &str, body: &str, needs_reply: bool) -> Result<InboxMessage> {
        let m = InboxMessage {
            id: util::new_id(),
            from_actor: from.into(),
            project: project.map(|s| s.to_string()),
            subject: subject.into(),
            body: body.into(),
            needs_reply,
            state: "open".into(),
            reply: None,
            replied_at: None,
            created_at: util::now(),
        };
        self.with(|c| {
            c.execute(
                "INSERT INTO inbox (id,from_actor,project,subject,body,needs_reply,state,created_at) VALUES (?1,?2,?3,?4,?5,?6,'open',?7)",
                params![m.id, m.from_actor, m.project, m.subject, m.body, m.needs_reply as i32, m.created_at],
            )?;
            Ok(())
        })?;
        Ok(m)
    }

    /// Inbox messages, newest first.
    pub fn list_inbox(&self, only_open: bool, limit: i64) -> Result<Vec<InboxMessage>> {
        self.with(|c| {
            let mut st = c.prepare(
                "SELECT id,from_actor,project,subject,body,needs_reply,state,reply,replied_at,created_at FROM inbox
                 WHERE (?1=0 OR state='open') ORDER BY created_at DESC LIMIT ?2",
            )?;
            let rows = st.query_map(params![only_open as i32, limit], row_inbox)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// One message.
    pub fn get_inbox(&self, id: &str) -> Result<Option<InboxMessage>> {
        self.with(|c| {
            Ok(c.query_row(
                "SELECT id,from_actor,project,subject,body,needs_reply,state,reply,replied_at,created_at FROM inbox WHERE id=?1",
                params![id],
                row_inbox,
            )
            .optional()?)
        })
    }

    /// Reply to and/or resolve a message.
    pub fn answer_inbox(&self, id: &str, reply: Option<&str>) -> Result<bool> {
        self.with(|c| {
            Ok(c.execute(
                "UPDATE inbox SET reply=COALESCE(?2,reply), replied_at=CASE WHEN ?2 IS NULL THEN replied_at ELSE ?3 END, state='resolved' WHERE id=?1",
                params![id, reply, util::now()],
            )? > 0)
        })
    }

    // ------------------------------------------------------------- requests

    /// Queue an operation for a person to approve.
    pub fn create_request(&self, actor: &str, capability: &str, args: &serde_json::Value, plan: Option<&serde_json::Value>, thread_id: Option<&str>, tool_use_id: Option<&str>) -> Result<AgentRequest> {
        let id = util::new_id();
        let now = util::now();
        self.with(|c| {
            c.execute(
                "INSERT INTO agent_requests (id,actor,capability,args,plan,state,thread_id,tool_use_id,created_at) VALUES (?1,?2,?3,?4,?5,'pending',?6,?7,?8)",
                params![id, actor, capability, args.to_string(), plan.map(|p| p.to_string()), thread_id, tool_use_id, now],
            )?;
            Ok(())
        })?;
        Ok(self.get_request(&id)?.expect("just inserted"))
    }

    /// One request.
    pub fn get_request(&self, id: &str) -> Result<Option<AgentRequest>> {
        self.with(|c| Ok(c.query_row(&format!("SELECT {REQ_COLS} FROM agent_requests WHERE id=?1"), params![id], row_request).optional()?))
    }

    /// Requests, optionally only pending, newest first.
    pub fn list_requests(&self, only_pending: bool, thread_id: Option<&str>, limit: i64) -> Result<Vec<AgentRequest>> {
        self.with(|c| {
            let mut st = c.prepare(&format!(
                "SELECT {REQ_COLS} FROM agent_requests WHERE (?1=0 OR state='pending') AND (?2 IS NULL OR thread_id=?2) ORDER BY created_at DESC LIMIT ?3"
            ))?;
            let rows = st.query_map(params![only_pending as i32, thread_id, limit], row_request)?;
            Ok(rows.collect::<std::result::Result<Vec<_>, _>>()?)
        })
    }

    /// Record a decision.
    pub fn decide_request(&self, id: &str, state: &str, decided_by: &str, result: Option<&serde_json::Value>, error: Option<&str>) -> Result<()> {
        self.with(|c| {
            c.execute(
                "UPDATE agent_requests SET state=?2, decided_by=?3, decided_at=?4, result=?5, error=?6 WHERE id=?1",
                params![id, state, decided_by, util::now(), result.map(|r| r.to_string()), error],
            )?;
            Ok(())
        })
    }
}

fn row_inbox(r: &rusqlite::Row) -> rusqlite::Result<InboxMessage> {
    Ok(InboxMessage {
        id: r.get(0)?,
        from_actor: r.get(1)?,
        project: r.get(2)?,
        subject: r.get(3)?,
        body: r.get(4)?,
        needs_reply: r.get::<_, i32>(5)? != 0,
        state: r.get(6)?,
        reply: r.get(7)?,
        replied_at: r.get(8)?,
        created_at: r.get(9)?,
    })
}
