//! Root-search gating: decides which of an extension's commands (if any) runs
//! live for a root-search query.
//!
//! Commands are found by fuzzy-searching their catalog entries (see
//! `providers::command::match_entries`) and invoked by entering their scope -
//! there is no prefix/alias trigger. The one exception is `always` commands,
//! which run on every keystroke like a built-in provider. The registry calls
//! [`gate`] before spawning any extension thread, so a non-`always` extension
//! costs nothing on the keystroke path (the fan-out invariant: at most one
//! thread per extension).

use crate::extensions::manifest::CommandSpec;

/// The query an extension receives after gating.
pub struct GatedQuery {
    /// The query text passed to the extension (the whole typed term).
    pub query: String,
    /// The launcher query exactly as typed (equals `query`).
    pub raw_query: String,
}

/// A gated query plus the command it resolved to.
pub struct GatedCommand {
    /// `CommandSpec.name` of the matched command.
    pub command: String,
    pub gated: GatedQuery,
}

/// Resolves the single command (if any) a root-search keystroke invokes.
///
/// Only `always = true` commands run in root search; the first one receives the
/// raw query (min length 1). Every other command runs only in its scope. When
/// an extension has no `always` command, this returns `None` and the registry
/// spawns no thread for it.
pub fn gate(commands: &[CommandSpec], raw_query: &str) -> Option<GatedCommand> {
    let cmd = commands.iter().find(|c| c.always)?;
    if raw_query.is_empty() || raw_query.len() < cmd.min_len().max(1) {
        return None;
    }
    Some(GatedCommand {
        command: cmd.name.clone(),
        gated: GatedQuery {
            query: raw_query.to_string(),
            raw_query: raw_query.to_string(),
        },
    })
}

/// Builds the gated query for an entered command mode: the whole query is the
/// command's input, gated only by `min_query_len` (empty query = browse state,
/// always allowed).
pub fn gate_scoped(cmd: &CommandSpec, query: &str) -> Option<GatedCommand> {
    if !query.is_empty() && query.len() < cmd.min_len() {
        return None;
    }
    Some(GatedCommand {
        command: cmd.name.clone(),
        gated: GatedQuery {
            query: query.to_string(),
            raw_query: query.to_string(),
        },
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cmd(name: &str, keywords: &[&str], min: usize, always: bool) -> CommandSpec {
        CommandSpec {
            name: name.to_string(),
            title: name.to_string(),
            description: String::new(),
            mode: "scope".to_string(),
            keywords: keywords.iter().map(|s| s.to_string()).collect(),
            min_query_len: min,
            always,
            chip: String::new(),
            placeholder: String::new(),
            kind: None,
            icon: None,
            opens_form: false,
            default_shortcut: None,
        }
    }

    #[test]
    fn non_always_never_runs_in_root() {
        let cmds = [cmd("emoji", &["emoji", "em"], 0, false)];
        // No prefix trigger: even a query equal to a former prefix runs nothing.
        assert!(gate(&cmds, "emoji smi").is_none());
        assert!(gate(&cmds, "emoji").is_none());
    }

    #[test]
    fn always_command_passes_raw_query() {
        let cmds = [cmd("conv", &["convert"], 0, true)];
        let g = gate(&cmds, "10 usd").unwrap();
        assert_eq!(g.command, "conv");
        assert_eq!(g.gated.query, "10 usd");
        assert_eq!(g.gated.raw_query, "10 usd");
    }

    #[test]
    fn always_honors_min_len_and_empty() {
        let cmds = [cmd("conv", &["convert"], 3, true)];
        assert!(gate(&cmds, "").is_none());
        assert!(gate(&cmds, "ab").is_none());
        assert!(gate(&cmds, "abc").is_some());
    }

    #[test]
    fn scoped_gate_honors_min_len_but_allows_browse() {
        let c = cmd("issues", &["ghi"], 3, false);
        assert!(gate_scoped(&c, "").is_some());
        assert!(gate_scoped(&c, "ab").is_none());
        assert_eq!(gate_scoped(&c, "abc").unwrap().gated.query, "abc");
        assert_eq!(gate_scoped(&c, "abc").unwrap().gated.raw_query, "abc");
    }
}
