# Secure Proxy Server

A production-ready HTTP/HTTPS proxy server with token-based authentication, built in Rust. Deploys easily to Render.com.

## Features

✅ **HTTP Proxying** - Forward standard HTTP requests  
✅ **HTTPS CONNECT Tunneling** - Full support for HTTPS sites via CONNECT method  
✅ **Token Authentication** - Secure access with configurable tokens  
✅ **Detailed Logging** - Comprehensive request/response logging with tracing  
✅ **Configuration File** - Easy setup via `config.toml`  
✅ **One-Click Render Deploy** - Simple deployment to Render.com

## Configuration

Edit `config.toml` to configure your proxy:

```toml
[server]
port = 8080
host = "0.0.0.0"  # Use "0.0.0.0" for cloud deployment

[tokens]
user1 = "token-for-alice"
user2 = "token-for-bob"
admin = "super-secret-admin-token"
```

## Local Development

```bash
# Run in development mode
cargo run

# Production build
cargo build --release
./target/release/secure-proxy

# With debug logging
RUST_LOG=debug cargo run
```

## Deploy to Render

### Quick Deploy (Recommended)

1. **Push to GitHub:**
```bash
git init
git add .
git commit -m "Initial commit"
git remote add origin https://github.com/yourusername/secure-proxy.git
git push -u origin main
```

2. **Deploy on Render:**
   - Go to [Render Dashboard](https://dashboard.render.com/)
   - Click "New +" → "Web Service"
   - Connect your GitHub repository
   - Render will automatically detect the `render.yaml` and deploy

3. **Configure Tokens (Important):**
   - In Render dashboard, go to your service settings
   - Update `config.toml` with strong tokens before deploying
   - Or use environment variables in Render dashboard

### Manual Render Setup

If not using `render.yaml`:

1. Create a new Web Service on Render
2. Connect your repository
3. Configure:
   - **Environment**: Docker
   - **Plan**: Starter ($7/month) or Free
   - **Dockerfile Path**: `./Dockerfile`
   - **Health Check Path**: `/`

4. Deploy!

Your proxy will be available at: `https://your-service-name.onrender.com`

## Using the Proxy

### Configure Your Client

**cURL (with Render deployment):**
```bash
# HTTP request
curl -x https://your-service.onrender.com:443 \
  -H "X-Proxy-Token: token-for-alice" \
  http://example.com

# HTTPS request (CONNECT tunnel)
curl -x https://your-service.onrender.com:443 \
  -H "X-Proxy-Token: token-for-alice" \
  https://www.google.com
```

**Python:**
```python
import requests

proxies = {
    'http': 'https://your-service.onrender.com',
    'https': 'https://your-service.onrender.com'
}

headers = {
    'X-Proxy-Token': 'token-for-alice'
}

# Works for both HTTP and HTTPS
response = requests.get('https://api.github.com', 
                       proxies=proxies, 
                       headers=headers)
```

**Local Testing:**
```bash
# Configure client to use:
# HTTP Proxy: 127.0.0.1:8080
# HTTPS Proxy: 127.0.0.1:8080

curl -x http://127.0.0.1:8080 \
  -H "X-Proxy-Token: token-for-alice" \
  https://www.google.com
```

## Security Considerations

1. **Use Strong Tokens** - Generate random tokens:
   ```bash
   openssl rand -hex 32
   ```

2. **Keep Tokens Secret** - Never commit real tokens to git. Use environment variables in production

3. **HTTPS Only** - Render provides HTTPS automatically for your proxy

4. **Rate Limiting** - Consider adding rate limiting for production use

5. **Rotate Tokens** - Regularly update authentication tokens

## Troubleshooting

**Port already in use (local):**
```bash
# Find process using port 8080
netstat -ano | findstr :8080
# Kill it (Windows)
taskkill /PID <PID> /F
```

**Token rejected:**
- Verify token matches exactly in config.toml
- Check logs in Render dashboard
- For local: `RUST_LOG=debug cargo run`

**Connection issues on Render:**
- Check Render service logs in dashboard
- Verify service is running (check health status)
- Ensure config.toml has `host = "0.0.0.0"`

## Log Levels

Set `RUST_LOG` environment variable in Render dashboard:

- `RUST_LOG=error` - Errors only
- `RUST_LOG=warn` - Warnings and errors
- `RUST_LOG=info` - Standard operation (default)
- `RUST_LOG=debug` - Detailed debugging
- `RUST_LOG=trace` - Very verbose

## Render Pricing

- **Free Tier**: Available but may spin down after inactivity
- **Starter Plan**: $7/month - Always on, better performance
- **Pro Plan**: $19/month - Enhanced resources

## License

MIT
