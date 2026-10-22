-- Terminal sessions and delegated tasks were a way to launch other people's
-- coding agents (Claude Code, OpenCode, a shell) from devd. devd has its own team
-- now, and the feature is gone.
DROP TABLE IF EXISTS agent_tasks;
DROP TABLE IF EXISTS agent_sessions;
