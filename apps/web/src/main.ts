/**
 * Browser app entrypoint for Vite.
 *
 * This file is imported by index.html and serves as the root module
 * for the browser bundle. It imports only shared frontend packages
 * and never references Tauri or desktop-only APIs.
 */

import { createWebAppConfig } from "./config.js";
import "./App";

const config = createWebAppConfig();

// Export config for debugging and dev tooling.
export { config as webConfig };
