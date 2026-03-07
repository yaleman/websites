import path from "node:path";
import { defineConfig } from "@rspack/cli";

export default defineConfig({
  entry: {
    editor: "./assets/admin/editor.ts",
  },
  output: {
    path: path.resolve(__dirname, "admin-ui-assets"),
    filename: "[name].js",
    cssFilename: "[name].css",
  },
  experiments: {
    css: true,
  },
  mode: "production",
});
