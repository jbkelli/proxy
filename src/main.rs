use hyper::service::{make_service_fn, service_fn};
use hyper::upgrade::Upgraded;
use hyper::{Body, Method, Request, Response, Client, Server};
use serde::Deserialize;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fs;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio::net::TcpStream;
use tracing::{info, warn, error, debug, instrument};

use base64::Engine as _;
use base64::engine::general_purpose::STANDARD as BASE64;
use hyper::header::{PROXY_AUTHORIZATION, PROXY_AUTHENTICATE};

#[derive(Debug, Deserialize)]
struct Config {
    server: ServerConfig,
    users: HashMap<String, String>, // username -> password
}

#[derive(Debug, Deserialize)]
struct ServerConfig {
    port: u16,
    host: String,
}

impl Config {
    fn load(path: &str) -> Result<Self, Box<dyn std::error::Error>> {
        info!("Loading configuration from: {}", path);
        let contents = fs::read_to_string(path)?;
        debug!(
            "Configuration file read successfully, {} bytes",
            contents.len()
        );
        let config: Config = toml::from_str(&contents)?;
        info!("Configuration parsed successfully");
        Ok(config)
    }

    fn is_valid_basic(&self, header: Option<&hyper::header::HeaderValue>) -> bool {
        if let Some(value) = header {
            if let Ok(v) = value.to_str() {
                let parts: Vec<&str> = v.split_whitespace().collect();
                if parts.len() == 2 && parts[0].eq_ignore_ascii_case("Basic") {
                    if let Ok(decoded) = BASE64.decode(parts[1]) {
                        if let Ok(creds) = String::from_utf8(decoded) {
                            if let Some((user, pass)) = creds.split_once(':') {
                                if let Some(stored) = self.users.get(user) {
                                    let ok = stored == pass;
                                    if ok {
                                        info!("✅ Proxy auth successful for user '{}'", user);
                                    } else {
                                        warn!("❌ Proxy auth wrong password for user '{}'", user);
                                    }
                                    return ok;
                                } else {
                                    warn!("❌ Proxy auth unknown user '{}'", user);
                                }
                            } else {
                                warn!("❌ Proxy auth creds missing ':' separator");
                            }
                        } else {
                            warn!("❌ Proxy auth creds not UTF-8");
                        }
                    } else {
                        warn!("❌ Proxy auth base64 decode failed");
                    }
                } else {
                    warn!("❌ Proxy auth header is not Basic");
                }
            } else {
                warn!("❌ Proxy auth header contains invalid UTF-8");
            }
        } else {
            warn!("❌ No Proxy-Authorization header provided");
        }
        false
    }
}

fn unauthorized_response() -> Response<Body> {
    // 407 with Proxy-Authenticate as required by spec
    Response::builder()
        .status(407)
        .header(PROXY_AUTHENTICATE, r#"Basic realm="Secure Proxy""#)
        .body(Body::from("Proxy authentication required"))
        .unwrap()
}

#[instrument(skip(req, config), fields(method = %req.method(), uri = %req.uri()))]
async fn handle_request(
    req: Request<Body>,
    config: Arc<Config>,
) -> Result<Response<Body>, Infallible> {
    info!("📨 Incoming request: {} {}", req.method(), req.uri());
    debug!("Request headers: {:?}", req.headers());

    // Health check endpoint (no auth required)
    if req.method() == Method::GET && req.uri().path() == "/health" {
        return Ok(Response::builder()
            .status(200)
            .body(Body::from("OK"))
            .unwrap());
    }

    // Require Proxy-Authorization for ALL requests (HTTP + CONNECT)
    let auth_header = req.headers().get(PROXY_AUTHORIZATION);
    if !config.is_valid_basic(auth_header) {
        warn!("🚫 Rejecting request due to invalid/missing proxy credentials");
        return Ok(unauthorized_response());
    }

    // Handle HTTPS CONNECT method vs normal HTTP
    if req.method() == Method::CONNECT {
        info!("Routing to HTTPS CONNECT handler");
        handle_connect(req).await
    } else {
        info!("Routing to HTTP proxy handler");
        handle_http(req).await
    }
}

#[instrument(skip(req), fields(uri = %req.uri()))]
async fn handle_http(req: Request<Body>) -> Result<Response<Body>, Infallible> {
    info!("🌐 Forwarding HTTP request to: {}", req.uri());
    let client = Client::new();
    match client.request(req).await {
        Ok(response) => {
            info!(
                "✅ HTTP request forwarded successfully, status: {}",
                response.status()
            );
            debug!("Response headers: {:?}", response.headers());
            Ok(response)
        }
        Err(err) => {
            error!("❌ HTTP proxy error: {}", err);
            Ok(Response::builder()
                .status(500)
                .body(Body::from(format!("Proxy error: {}", err)))
                .unwrap())
        }
    }
}

#[instrument(skip(req), fields(uri = %req.uri()))]
async fn handle_connect(mut req: Request<Body>) -> Result<Response<Body>, Infallible> {
    let uri_str = req.uri().to_string();

    // Extract host:port from URI
    let mut target = if uri_str.contains("://") {
        if let Ok(parsed_uri) = uri_str.parse::<hyper::Uri>() {
            if let Some(authority) = parsed_uri.authority() {
                authority.to_string()
            } else {
                warn!("⚠️ No authority in URI: {}", uri_str);
                uri_str
            }
        } else {
            warn!("⚠️ Failed to parse URI: {}", uri_str);
            uri_str
        }
    } else {
        uri_str
    };

    // Add default port 443 if no port specified
    if !target.contains(':') {
        debug!("No port specified for CONNECT, defaulting to 443");
        target = format!("{}:443", target);
    }

    info!("🔐 Handling HTTPS CONNECT request to: {}", target);
    debug!(
        "CONNECT request details - URI: {}, Version: {:?}",
        req.uri(),
        req.version()
    );

    tokio::task::spawn(async move {
        match hyper::upgrade::on(&mut req).await {
            Ok(upgraded) => {
                info!("✅ Connection upgraded for CONNECT tunnel to {}", target);
                if let Err(e) = tunnel(upgraded, target).await {
                    error!("❌ Tunnel error: {}", e);
                }
            }
            Err(e) => {
                error!("❌ Upgrade error: {}", e);
            }
        }
    });

    Ok(Response::builder()
        .status(200)
        .body(Body::empty())
        .unwrap())
}

// Create a tunnel between client and target server
async fn tunnel(mut upgraded: Upgraded, target: String) -> std::io::Result<()> {
    info!("🔗 Establishing tunnel to {}", target);

    let mut server = TcpStream::connect(&target).await?;
    info!("✅ Connected to target server: {}", target);

    let (from_client, from_server) =
        tokio::io::copy_bidirectional(&mut upgraded, &mut server).await?;

    info!(
        "🔚 Tunnel closed: {} - {} bytes from client, {} bytes from server",
        target, from_client, from_server
    );

    Ok(())
}

#[tokio::main]
async fn main() {
    eprintln!("DEBUG: Rust main() started");
    println!("DEBUG: Rust main() started");
    std::io::Write::flush(&mut std::io::stdout()).ok();
    std::io::Write::flush(&mut std::io::stderr()).ok();

    println!("=== STARTING PROXY SERVER ===");
    println!("Current directory: {:?}", std::env::current_dir());
    println!("Checking for config.toml...");

    if std::path::Path::new("config.toml").exists() {
        println!("✓ config.toml found!");
    } else {
        eprintln!("✗ config.toml NOT FOUND in current directory!");
        println!("Files in current directory:");
        if let Ok(entries) = std::fs::read_dir(".") {
            for entry in entries.flatten() {
                println!("  - {}", entry.path().display());
            }
        }
    }

    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(true)
        .with_line_number(true)
        .init();

    info!("🚀 Secure proxy server starting...");

    println!("Loading config from config.toml...");
    let config = match Config::load("config.toml") {
        Ok(cfg) => {
            println!("Config loaded successfully!");
            Arc::new(cfg)
        }
        Err(e) => {
            eprintln!("❌ Failed to load config.toml: {e:?}");
            error!("❌ Failed to load config.toml: {e:?}");
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            std::process::exit(1);
        }
    };

    let port = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse::<u16>().ok())
        .unwrap_or(config.server.port);

    info!("📍 Host: {}", config.server.host);
    info!(
        "🔌 Port: {} {}",
        port,
        if std::env::var("PORT").is_ok() {
            "(from PORT env var)"
        } else {
            "(from config)"
        }
    );
    info!("🔑 Loaded {} user(s)", config.users.len());
    debug!("Users: {:?}", config.users.keys().collect::<Vec<_>>());
    info!("✅ Configuration loaded successfully");

    let addr_str = format!("{}:{}", config.server.host, port);
    println!("About to bind to: {}", addr_str);

    let addr: SocketAddr = match addr_str.parse() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("❌ Failed to parse server address '{}': {}", addr_str, e);
            error!(
                "❌ Failed to parse server address '{}': {}",
                addr_str, e
            );
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            std::process::exit(1);
        }
    };

    let config_clone = config.clone();
    let make_svc = make_service_fn(move |_conn| {
        let config = config_clone.clone();
        async move {
            Ok::<_, Infallible>(service_fn(move |req| {
                let config = config.clone();
                handle_request(req, config)
            }))
        }
    });

    info!("Attempting to bind to {}", addr);
    println!("Attempting to bind to {}", addr);
    let server = Server::bind(&addr).serve(make_svc);

    info!("🎯 Proxy server listening on http://{}", addr);
    println!("✅ Server successfully bound and listening on http://{}", addr);
    info!("🌐 Ready to proxy HTTP and HTTPS requests with proxy authentication");

    if let Err(e) = server.await {
        error!("❌ Server error: {}", e);
        std::process::exit(1);
    }
}
