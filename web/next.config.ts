import type { NextConfig } from "next";
import path from "node:path";

const nextConfig: NextConfig = {
  reactStrictMode: true,
  // Don't let `next dev` write an AGENTS.md into web/; the repo has its own.
  agentRules: false,
  // Monorepo: the workspace root holds the lockfile and the linked @blockula/sdk.
  turbopack: { root: path.join(__dirname, "..") },
  outputFileTracingRoot: path.join(__dirname, ".."),
};

export default nextConfig;
