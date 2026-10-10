import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// `npm run dev` talks to `cargo run -- server --demo` on :8080.
export default defineConfig({
  plugins: [react()],
  server: { proxy: { "/api": "http://127.0.0.1:8080", "/healthz": "http://127.0.0.1:8080" } },
});
