import React from "react";
import ReactDOM from "react-dom/client";
import App from "./App";
import { ErrorBoundary } from "./components/ErrorBoundary";
import { applyTheme, currentTheme } from "./theme";
import "./styles.css";

applyTheme(currentTheme());

const root = document.getElementById("root");
if (!root) throw new Error("no #root element");

ReactDOM.createRoot(root).render(
  <React.StrictMode>
    <ErrorBoundary>
      <App />
    </ErrorBoundary>
  </React.StrictMode>,
);
