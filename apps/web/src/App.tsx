/**
 * React application entrypoint for OpenSymphony web client.
 *
 * This file mounts the React root and renders the AppShell
 * which contains all navigation surfaces and pages.
 */

import { StrictMode } from "react";
import { createRoot } from "react-dom/client";
import { AppShell } from "./components/AppShell";
import "./styles/global.css";

const root = document.getElementById("root");
if (!root) {
  throw new Error("Failed to find #root element");
}

createRoot(root).render(
  <StrictMode>
    <AppShell />
  </StrictMode>,
);
