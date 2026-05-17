# Web Client Deployment

Two deployment modes are supported for the OpenSymphony web client:

1. **Gateway-Served** (default) - The gateway serves the built web app
   static assets under `/app/*`. Used for local and external gateway modes.
2. **Separately Deployed** - The web app is hosted independently and
   connects to a gateway via a configurable base URL.

## Prerequisites

- Node.js 18+ with npm
- OpenSymphony Gateway running (for gateway-served mode)

## Building the Web App

```bash
cd apps/web
npm install
npm run build
```

The build outputs static assets to `apps/web/dist/` with cache-busted
filenames (e.g. `assets/main-a1b2c3d4.js`).

## Mode 1: Gateway-Served Deployment

The gateway can serve the built web app directly. Configure the gateway
to point at the `apps/web/dist/` directory:

### Rust (GatewayServer)

```rust
let server = GatewayServer::new(store)
    .with_web_assets("apps/web/dist");
```

The web app will be available at `/app/` on the gateway host.
API calls target the same origin by default.

### Build Configuration

No special build flags are needed. The default `VITE_APP_BASE_PATH=/app/`
matches the gateway route prefix.

## Mode 2: Separately Deployed

Deploy the contents of `apps/web/dist/` to any static file host
(nginx, Apache, CDN, etc.) and configure the gateway URL.

### Build Configuration

```bash
# Point at your gateway URL
export VITE_GATEWAY_URL=https://gateway.example.com

# Set base path to root (adjust for sub-path deployments)
export VITE_APP_BASE_PATH=/

npm run build
```

### Environment Variables

| Variable | Description | Default |
|----------|-------------|---------|
| `VITE_GATEWAY_URL` | Gateway base URL for API calls | Same origin |
| `VITE_APP_BASE_PATH` | Base path for static assets | `/app/` |
| `VITE_DEV_GATEWAY_URL` | Dev-server proxy target | `http://127.0.0.1:3000` |

See `apps/web/.env.example` for documented defaults.

### nginx Example

```nginx
server {
    listen 80;
    server_name web.example.com;
    root /path/to/apps/web/dist;

    location / {
        try_files $uri $uri/ /index.html;
    }
}
```

## Local Development

```bash
cd apps/web
# Optional: set VITE_DEV_GATEWAY_URL to your local gateway
npm run dev
```

The Vite dev server runs on `http://localhost:5173` and proxies `/api`,
/ws`, and `/events` to the local gateway (default: `http://127.0.0.1:3000`).

## Cache-Busted Assets

Vite automatically hashes asset filenames for long-term caching.
The `index.html` file references the correct hashed filenames, so it
should be served with `Cache-Control: no-cache` while static assets
can use `Cache-Control: max-age=31536000, immutable`.

## Tauri API Isolation

The web build excludes all Tauri and desktop-only dependencies.
This is enforced by:

- Separate entry points (`apps/web/src/main.ts` vs `apps/desktop/src/index.ts`)
- Vite resolve aliases that point only to shared packages
- Build smoke tests that verify no Tauri references in the bundle

## Smoke Tests

```bash
# Build first
cd apps/web && npm run build

# Run all tests (includes web build smoke tests)
npm test
```
