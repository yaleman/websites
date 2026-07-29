import path from "node:path";
import { defineConfig } from "@rspack/cli";

export default defineConfig({
	entry: {
		admin: "./assets/admin/admin.ts",
		editor: "./assets/admin/editor.ts",
		mass_asset_import: "./assets/admin/mass_asset_import.ts",
		remediation: "./assets/admin/remediation.ts",
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
			{
				test: /\.css$/,
				use: ["postcss-loader"],
				type: "css",
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
	resolve: {
		extensions: [".ts", ".js"],
	},
});
