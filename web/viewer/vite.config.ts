import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { defineConfig, type Plugin } from "vite";

function gravimeraWastelandAssets(): Plugin {
  const configDir = path.dirname(fileURLToPath(import.meta.url));
  const repoRoot = path.resolve(configDir, "../..");
  const wastelandDir = path.join(repoRoot, "assets", "scene_wasteland");
  const routePrefix = "/assets/scene_wasteland/";

  let outDir = path.join(configDir, "dist");

  function tryCopyWastelandAssets() {
    // Best-effort: make the default autoload assets available in the production bundle too.
    try {
      const destDir = path.join(outDir, "assets", "scene_wasteland");
      fs.mkdirSync(destDir, { recursive: true });
      for (const name of ["scene.grav", "terrain.grav"]) {
        fs.copyFileSync(path.join(wastelandDir, name), path.join(destDir, name));
      }
    } catch (err) {
      console.warn(`[gravimera] Failed to copy scene_wasteland assets: ${String(err)}`);
    }
  }

  return {
    name: "gravimera-wasteland-assets",
    configResolved(config) {
      outDir = path.resolve(config.root, config.build.outDir);
    },
    configureServer(server) {
      server.middlewares.use((req, res, next) => {
        const url = req.url?.split("?", 1)[0] ?? "";
        if (!url.startsWith(routePrefix)) return next();

        const rel = decodeURIComponent(url.slice(routePrefix.length));
        if (!rel || rel.includes("..") || rel.includes("\\") || path.isAbsolute(rel)) {
          res.statusCode = 400;
          res.end("Bad path");
          return;
        }

        const filePath = path.join(wastelandDir, rel);
        if (!filePath.startsWith(wastelandDir)) {
          res.statusCode = 400;
          res.end("Bad path");
          return;
        }

        fs.stat(filePath, (err, stat) => {
          if (err || !stat.isFile()) return next();
          res.statusCode = 200;
          res.setHeader("Content-Type", "application/octet-stream");
          res.setHeader("Content-Length", String(stat.size));
          fs.createReadStream(filePath).pipe(res);
        });
      });
    },
    closeBundle() {
      tryCopyWastelandAssets();
    },
  };
}

export default defineConfig({
  plugins: [gravimeraWastelandAssets()],
  server: {
    port: 5173,
    strictPort: true,
    // This viewer is commonly accessed via arbitrary hostnames / IPs on LAN or a server.
    // Disable Vite's host allowlist.
    allowedHosts: true,
  },
  preview: {
    // Match the dev server host validation policy.
    allowedHosts: true,
  },
});
