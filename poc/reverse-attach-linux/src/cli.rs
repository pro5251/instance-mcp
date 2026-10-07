//! Command-line flags with the macOS daemon's names (instance-mcp#32 item 8), so
//! Connect / Remote instructions apply to every platform unchanged. They are an
//! addition: with no flags the node reads its environment exactly as before, and a
//! flag wins over the variable it corresponds to.

use std::sync::OnceLock;

/// What the command line asks for.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Invocation {
    Run(Flags),
    Version,
    Help,
}

/// Flags as given; `None` / empty means "not on the command line".
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Flags {
    host: Option<String>,
    port: Option<u16>,
    path: Option<String>,
    allow_logins: Vec<String>,
    token: Option<String>,
    token_file: Option<String>,
    insecure_local: bool,
    upstreams: Vec<String>,
    no_attach: bool,
    no_grant_persistence: bool,
    public_url: Option<String>,
}

/// Options that have no environment variable; read by the HTTP server.
#[derive(Debug)]
pub(crate) struct Options {
    pub(crate) mcp_path: String,
    pub(crate) attach: bool,
}

static OPTIONS: OnceLock<Options> = OnceLock::new();

pub(crate) fn options() -> &'static Options {
    OPTIONS.get_or_init(|| Options {
        mcp_path: "/mcp".to_string(),
        attach: true,
    })
}

pub(crate) const USAGE: &str = "\
USAGE: oab-instance-mcp [--host 127.0.0.1] [--port 8790] [--path /mcp]
                        [--allow-login <email>]... [--token <str> | --token-file <path>]
                        [--insecure-local] [--upstream <name=url>]...
                        [--no-attach] [--no-grant-persistence] [--public-url <https://…/mcp>]
                        [--version] [--help]

Every flag also has an environment variable (BIND, MCP_ALLOW_LOGIN, MCP_TOKEN,
MCP_TOKEN_FILE, MCP_INSECURE_LOCAL, MCP_UPSTREAM, MCP_GRANTS_FILE=off); a flag wins.
Auth: at least one of --allow-login / --token / --token-file, unless --insecure-local.";

pub(crate) fn parse(args: &[String]) -> Result<Invocation, String> {
    let mut flags = Flags::default();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| format!("missing value for {name}"))
        };
        match arg.as_str() {
            "--host" => flags.host = Some(value(arg)?),
            "--port" => {
                let v = value(arg)?;
                flags.port = Some(v.parse().map_err(|_| format!("bad port: {v}"))?);
            }
            "--path" => {
                let v = value(arg)?;
                if !v.starts_with('/') || v == "/attach" || v.starts_with("/attach/") {
                    return Err(format!(
                        "--path wants an absolute path other than /attach, got {v}"
                    ));
                }
                flags.path = Some(v);
            }
            "--allow-login" => flags.allow_logins.push(value(arg)?),
            "--token" => flags.token = Some(value(arg)?),
            "--token-file" => flags.token_file = Some(value(arg)?),
            "--insecure-local" => flags.insecure_local = true,
            "--upstream" => {
                let v = value(arg)?;
                match v.split_once('=') {
                    Some((name, url)) if !name.is_empty() && url.starts_with("http://") => {}
                    _ => {
                        return Err(format!(
                            "--upstream wants name=http://host:port/path, got {v}"
                        ))
                    }
                }
                flags.upstreams.push(v);
            }
            "--no-attach" => flags.no_attach = true,
            "--no-grant-persistence" => flags.no_grant_persistence = true,
            "--public-url" => flags.public_url = Some(value(arg)?),
            "--version" => return Ok(Invocation::Version),
            "-h" | "--help" => return Ok(Invocation::Help),
            other => return Err(format!("unknown flag {other}")),
        }
    }
    if flags.token.is_some() && flags.token_file.is_some() {
        return Err("give --token or --token-file, not both".to_string());
    }
    Ok(Invocation::Run(flags))
}

impl Flags {
    /// Environment variables this command line sets, as (name, value). `bind_env` is
    /// the current `BIND`, so `--port` alone keeps a host given there.
    pub(crate) fn env_overrides(&self, bind_env: Option<&str>) -> Vec<(&'static str, String)> {
        let mut out = Vec::new();
        if self.host.is_some() || self.port.is_some() {
            let (env_host, env_port) = bind_env
                .and_then(|b| b.rsplit_once(':'))
                .map(|(h, p)| (h.to_string(), p.to_string()))
                .unwrap_or_else(|| ("127.0.0.1".to_string(), "8790".to_string()));
            let host = self.host.clone().unwrap_or(env_host);
            let port = self.port.map(|p| p.to_string()).unwrap_or(env_port);
            out.push(("BIND", format!("{host}:{port}")));
        }
        if !self.allow_logins.is_empty() {
            out.push(("MCP_ALLOW_LOGIN", self.allow_logins.join(",")));
        }
        // A token flag replaces both token variables, so an inherited one cannot win.
        if let Some(token) = &self.token {
            out.push(("MCP_TOKEN", token.clone()));
            out.push(("MCP_TOKEN_FILE", String::new()));
        }
        if let Some(file) = &self.token_file {
            out.push(("MCP_TOKEN", String::new()));
            out.push(("MCP_TOKEN_FILE", file.clone()));
        }
        if self.insecure_local {
            out.push(("MCP_INSECURE_LOCAL", "1".to_string()));
        }
        if !self.upstreams.is_empty() {
            out.push(("MCP_UPSTREAM", self.upstreams.join(",")));
        }
        if self.no_grant_persistence {
            out.push(("MCP_GRANTS_FILE", "off".to_string()));
        }
        out
    }

    /// Apply this command line: environment overrides first, then the options that
    /// have no variable. Call once at start-up, before any thread starts.
    pub(crate) fn apply(&self) {
        let bind = std::env::var("BIND").ok();
        for (name, value) in self.env_overrides(bind.as_deref()) {
            std::env::set_var(name, value);
        }
        let _ = OPTIONS.set(Options {
            mcp_path: self.path.clone().unwrap_or_else(|| "/mcp".to_string()),
            attach: !self.no_attach,
        });
        if let Some(url) = &self.public_url {
            eprintln!("public url: {url}");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Vec<String> {
        list.iter().map(|s| s.to_string()).collect()
    }

    fn run(list: &[&str]) -> Flags {
        match parse(&args(list)).expect("parses") {
            Invocation::Run(f) => f,
            other => panic!("expected Run, got {other:?}"),
        }
    }

    #[test]
    fn no_flags_change_nothing() {
        let f = run(&[]);
        assert_eq!(f, Flags::default());
        assert!(f.env_overrides(Some("0.0.0.0:1")).is_empty());
    }

    #[test]
    fn host_and_port_build_bind_and_keep_the_other_half_from_the_environment() {
        let both = run(&["--host", "127.0.0.2", "--port", "8795"]);
        assert_eq!(
            both.env_overrides(None),
            vec![("BIND", "127.0.0.2:8795".to_string())]
        );
        let port_only = run(&["--port", "9000"]);
        assert_eq!(
            port_only.env_overrides(Some("10.0.0.5:8790")),
            vec![("BIND", "10.0.0.5:9000".to_string())]
        );
        assert_eq!(
            port_only.env_overrides(None),
            vec![("BIND", "127.0.0.1:9000".to_string())]
        );
    }

    #[test]
    fn repeatable_flags_join_like_their_variables() {
        let f = run(&[
            "--allow-login",
            "a@x.com",
            "--allow-login",
            "b@x.com",
            "--upstream",
            "browser=http://127.0.0.1:8794/mcp",
            "--upstream",
            "x=http://127.0.0.1:1/mcp",
        ]);
        let env = f.env_overrides(None);
        assert!(env.contains(&("MCP_ALLOW_LOGIN", "a@x.com,b@x.com".to_string())));
        assert!(env.contains(&(
            "MCP_UPSTREAM",
            "browser=http://127.0.0.1:8794/mcp,x=http://127.0.0.1:1/mcp".to_string()
        )));
    }

    #[test]
    fn a_token_flag_clears_the_other_token_variable() {
        let env = run(&["--token-file", "/t"]).env_overrides(None);
        assert!(env.contains(&("MCP_TOKEN", String::new())));
        assert!(env.contains(&("MCP_TOKEN_FILE", "/t".to_string())));
        let env = run(&["--token", "s3"]).env_overrides(None);
        assert!(env.contains(&("MCP_TOKEN", "s3".to_string())));
        assert!(env.contains(&("MCP_TOKEN_FILE", String::new())));
    }

    #[test]
    fn switches_map_to_their_variables() {
        let env = run(&["--insecure-local", "--no-grant-persistence"]).env_overrides(None);
        assert!(env.contains(&("MCP_INSECURE_LOCAL", "1".to_string())));
        assert!(env.contains(&("MCP_GRANTS_FILE", "off".to_string())));
    }

    #[test]
    fn version_help_and_errors() {
        assert_eq!(parse(&args(&["--version"])), Ok(Invocation::Version));
        assert_eq!(parse(&args(&["-h"])), Ok(Invocation::Help));
        assert!(parse(&args(&["--bogus"]))
            .unwrap_err()
            .contains("unknown flag"));
        assert!(parse(&args(&["--port"]))
            .unwrap_err()
            .contains("missing value"));
        assert!(parse(&args(&["--port", "x"]))
            .unwrap_err()
            .contains("bad port"));
        assert!(parse(&args(&["--path", "/attach"])).is_err());
        assert!(parse(&args(&["--path", "mcp"])).is_err());
        assert!(parse(&args(&["--upstream", "nourl"])).is_err());
        assert!(parse(&args(&["--token", "a", "--token-file", "b"])).is_err());
    }
}
