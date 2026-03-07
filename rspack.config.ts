import path from "node:path";
import { defineConfig } from "@rspack/cli";

export default defineConfig({
  entry: {
    editor: "./assets/admin/editor.ts",
  },
  module: {
    rules: [
      {
        test: /\.ts$/,
        exclude: [/node_modules/],
        use: [
          {
            loader: "builtin:swc-loader",
            options: {
              jsc: {
                parser: {
                  syntax: "typescript",
                },
              },
            },
          },
        ],
      },
    ],
  },
  output: {
    path: path.resolve(__dirname, "admin-ui-assets"),
    filename: "[name].js",
    cssFilename: "[name].css",
  },
  performance: {
    hints: false,
  },
  experiments: {
    css: true,
  },
  mode: "production",
});
