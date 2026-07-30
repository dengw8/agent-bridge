import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import "./index.css";

const previewClient = import.meta.env.DEV && new URLSearchParams(location.search).has("preview")
  ? (await import("./tauri/preview")).previewClient
  : undefined;

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App {...(previewClient ? { client: previewClient } : {})} />
  </React.StrictMode>,
);
