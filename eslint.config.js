import js from "@eslint/js";
import tseslint from "@typescript-eslint/eslint-plugin";
import tsparser from "@typescript-eslint/parser";
import globals from "globals";

export default [
  js.configs.recommended,
  {
    files: ["src/**/*.{ts,tsx}"],
    languageOptions: {
      parser: tsparser,
      parserOptions: { project: "./tsconfig.json", ecmaVersion: 2022, sourceType: "module" },
      globals: { ...globals.browser },
    },
    plugins: { "@typescript-eslint": tseslint },
    rules: {
      // The base rules do not understand type-only declarations or ambient DOM
      // types, so the TypeScript-aware versions replace them.
      "no-unused-vars": "off",
      "no-undef": "off",

      // SPEC 0: `any` is banned outright, not merely discouraged.
      "@typescript-eslint/no-explicit-any": "error",
      "@typescript-eslint/no-unused-vars": ["error", { argsIgnorePattern: "^_" }],
      "@typescript-eslint/consistent-type-imports": "error",

      "no-restricted-syntax": [
        "error",
        {
          // SPEC 4: no network, ever. Enforced rather than trusted.
          selector: "CallExpression[callee.name='fetch']",
          message: "Aplikasi ini offline total (SPEC 4). Pakai protokol izul:// atau invoke().",
        },
        {
          selector: "NewExpression[callee.name=/^(WebSocket|EventSource|XMLHttpRequest)$/]",
          message: "Aplikasi ini offline total (SPEC 4).",
        },
      ],
    },
  },
  {
    // React owns what is on screen; this folder owns what is painted. Importing
    // React here would put a reconciliation pass inside the frame budget, so
    // SPEC 4's rule is enforced rather than trusted.
    files: ["src/viewport/**/*.ts"],
    rules: {
      "no-restricted-imports": [
        "error",
        {
          patterns: [
            {
              group: ["react", "react-dom", "react/*", "react-dom/*"],
              message: "src/viewport/ harus bebas React (SPEC 4).",
            },
          ],
        },
      ],
    },
  },
  {
    // The tile source is the one place allowed to call fetch, because the
    // izul:// custom protocol is how pixels reach the page at all (SPEC 6).
    // Confining the exception to one file keeps the offline rule enforceable
    // everywhere else.
    files: ["src/viewport/tileSource.ts"],
    rules: { "no-restricted-syntax": "off" },
  },
];
