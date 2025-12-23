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
        debug!("Configuration file read successfully, {} bytes", contents.len());
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
                    debug!("Valid token: {}...{}", &token_str[..token_str.len().min(4)], &token_str[token_str.len().saturating_sub(4)..]);
                } else {
                    warn!("❌ Invalid token provided: {}...{}", &token_str[..token_str.len().min(4)], &token_str[token_str.len().saturating_sub(4)..]);
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
async fn handle_request(req: Request<Body>, config: Arc<Config>) -> Result<Response<Body>, Infallible> {
    info!("📨 Incoming request: {} {}", req.method(), req.uri());
    debug!("Request headers: {:?}", req.headers());
    
    // Check for X-Proxy-Token header (for non-CONNECT requests)
    if req.method() != Method::CONNECT {
        debug!("Validating token for non-CONNECT request");
        let token = req.headers().get("X-Proxy-Token");
        if !config.is_valid_token(token) {
            warn!("🚫 Rejecting request due to invalid/missing token");
            return Ok(Response::builder()
                .status(403)
                .body(Body::from("Invalid or missing token"))
                .unwrap());
        }
    }

    // Handle HTTPS CONNECT method
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
            info!("✅ HTTP request forwarded successfully, status: {}", response.status());
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
    let target = req.uri().to_string();
    info!("🔐 Handling HTTPS CONNECT request to: {}", target);
    debug!("CONNECT request details - URI: {}, Version: {:?}", req.uri(), req.version());
    
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

    Ok(Response::new(Body::empty()))
}

// Create a tunnel between client and target server
async fn tunnel(mut upgraded: Upgraded, target: String) -> std::io::Result<()> {
    info!("🔗 Establishing tunnel to {}", target);
    
    // Connect to the target server
    let mut server = TcpStream::connect(&target).await?;
    info!("✅ Connected to target server: {}", target);
    
    // Bidirectional copy between client and server
    let (from_client, from_server) = tokio::io::copy_bidirectional(&mut upgraded, &mut server).await?;
    
    info!("🔚 Tunnel closed: {} - {} bytes from client, {} bytes from server", 
          target, from_client, from_server);
    
    Ok(())
}

#[tokio::main]
async fn main() {
    // Print to stdout BEFORE tracing init
    println!("=== STARTING PROXY SERVER ===");
    println!("Current directory: {:?}", std::env::current_dir());
    
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
            eprintln!("❌ Failed to load config.toml: {}", e);
            error!("❌ Failed to load config.toml: {}", e);
            std::process::exit(1);
        }
    };
    
    info!("📍 Host: {}", config.server.host);
    info!("🔌 Port: {}", config.server.port);
    info!("🔑 Loaded {} valid token(s)", config.tokens.len());
    debug!("Token users: {:?}", config.tokens.keys().collect::<Vec<_>>());
    info!("✅ Configuration loaded successfully");
    
    // Build server address
    let addr: SocketAddr = format!("{}:{}", config.server.host, config.server.port)
        .parse()
        .expect("Failed to parse server address");
    
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
    let server = Server::bind(&addr).serve(make_svc);
    
    info!("🎯 Proxy server listening on http://{}", addr);
    info!("📝 Send requests with 'X-Proxy-Token' header for authentication");
    info!("🌐 Ready to proxy HTTP and HTTPS requests");
    
    // Run the server
    if let Err(e) = server.await {
        error!("❌ Server error: {}", e);
        std::process::exit(1);
    }
}