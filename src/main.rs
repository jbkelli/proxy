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

#[derive(Debug, Deserialize)]
struct Config {
    server: ServerConfig,
    tokens: HashMap<String, String>,
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

    fn is_valid_token(&self, token: Option<&hyper::header::HeaderValue>) -> bool {
        if let Some(token_value) = token {
            if let Ok(token_str) = token_value.to_str() {
                let is_valid = self.tokens.values().any(|valid_token| valid_token == token_str);
                if is_valid {
                    info!("✅ Token validation successful");
                    debug!(
                        "Valid token: {}...{}",
                        &token_str[..token_str.len().min(4)],
                        &token_str[token_str.len().saturating_sub(4)..]
                    );
                } else {
                    warn!(
                        "❌ Invalid token provided: {}...{}",
                        &token_str[..token_str.len().min(4)],
                        &token_str[token_str.len().saturating_sub(4)..]
                    );
                }
                return is_valid;
            } else {
                warn!("❌ Token header contains invalid UTF-8");
            }
        } else {
            warn!("❌ No token provided in request");
        }
        false
    }
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

    // Require token for ALL requests, including CONNECT
    debug!("Validating token for {} request", req.method());
    let token = req.headers().get("X-Proxy-Token");
    if !config.is_valid_token(token) {
        warn!("🚫 Rejecting request due to invalid/missing token");
        return Ok(Response::builder()
            .status(403)
            .body(Body::from("Invalid or missing token"))
            .unwrap());
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
    // Some clients send "example.com:443", others send "https://example.com:443"
    let mut target = if uri_str.contains("://") {
        // Parse full URI and extract authority (host:port)
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
        // Already in host:port format
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

    // Return explicit 200 Connection Established response
    Ok(Response::builder()
        .status(200)
        .body(Body::empty())
        .unwrap())
}

// Create a tunnel between client and target server
async fn tunnel(mut upgraded: Upgraded, target: String) -> std::io::Result<()> {
    info!("🔗 Establishing tunnel to {}", target);

    // Connect to the target server
    let mut server = TcpStream::connect(&target).await?;
    info!("✅ Connected to target server: {}", target);

    // Bidirectional copy between client and server
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
    // Print IMMEDIATELY before anything else
    eprintln!("DEBUG: Rust main() started");
    println!("DEBUG: Rust main() started");
    std::io::Write::flush(&mut std::io::stdout()).ok();
    std::io::Write::flush(&mut std::io::stderr()).ok();

    // Print to stdout BEFORE tracing init
    println!("=== STARTING PROXY SERVER ===");
    println!("Current directory: {:?}", std::env::current_dir());
    println!("Checking for config.toml...");

    // Check if config file exists
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

    // Initialize tracing subscriber
    tracing_subscriber::fmt()
        .with_target(false)
        .with_thread_ids(true)
        .with_line_number(true)
        .init();

    info!("🚀 Secure proxy server starting...");

    // Load configuration
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

    // Override port with PORT environment variable if set
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
    info!("🔑 Loaded {} valid token(s)", config.tokens.len());
    debug!("Token users: {:?}", config.tokens.keys().collect::<Vec<_>>());
    info!("✅ Configuration loaded successfully");

    // Build server address
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

    // Create service
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

    // Build and run server
    info!("Attempting to bind to {}", addr);
    println!("Attempting to bind to {}", addr);
    let server = Server::bind(&addr).serve(make_svc);

    info!("🎯 Proxy server listening on http://{}", addr);
    println!("✅ Server successfully bound and listening on http://{}", addr);
    info!("📝 Send requests with 'X-Proxy-Token' header for authentication");
    info!("🌐 Ready to proxy HTTP and HTTPS requests");

    // Run the server
    if let Err(e) = server.await {
        error!("❌ Server error: {}", e);
        std::process::exit(1);
    }
}
