#[derive(Debug, Clone)]
struct ServiceConfig {
    bind: String,
    token: Option<String>,
}

impl Default for ServiceConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:8792".to_string(),
            token: None,
        }
    }
}

fn parse_args() -> ServiceConfig {
    let mut cfg = ServiceConfig::default();
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--bind" => {
                if let Some(v) = args.next() {
                    cfg.bind = v;
                } else {
                    eprintln!("`--bind` expects an address like `127.0.0.1:8792`.");
                    std::process::exit(2);
                }
            }
            "--token" => {
                if let Some(v) = args.next() {
                    if !v.trim().is_empty() {
                        cfg.token = Some(v);
                    }
                } else {
                    eprintln!("`--token` expects a bearer token string.");
                    std::process::exit(2);
                }
            }
            "--help" | "-h" => {
                println!(
                    "gravimera_intelligence_service\n\
                     \n\
                     Options:\n\
                       --bind 127.0.0.1:8792   Bind address (default: 127.0.0.1:8792)\n\
                       --token <token>         Require Authorization: Bearer <token>\n"
                );
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown arg: {other}");
                std::process::exit(2);
            }
        }
    }
    cfg
}

fn main() {
    let cfg = parse_args();
    if let Err(err) = gravimera::intelligence::service::run_intelligence_service_blocking(
        cfg.bind.as_str(),
        cfg.token,
    ) {
        eprintln!("{err}");
        std::process::exit(1);
    }
}
