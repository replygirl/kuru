import { defineConfig } from "vitepress";
import llmstxt from "vitepress-plugin-llms";

const base = process.env.KURU_DOCS_BASE || "/kuru/";

export default defineConfig({
  title: "Kuru",
  description:
    "A Rust terminal harness built around persistent peers, private memories, and fluid relationships.",
  lang: "en-US",
  base,
  cleanUrls: true,
  appearance: "dark",
  lastUpdated: true,
  sitemap: { hostname: `https://replygirl.github.io${base}` },
  head: [
    ["meta", { name: "theme-color", content: "#0f131e" }],
    ["link", { rel: "icon", type: "image/svg+xml", href: `${base}favicon.svg` }],
  ],
  vite: {
    plugins: [llmstxt({ excludeIndexPage: false })],
    resolve: { preserveSymlinks: true },
  },
  themeConfig: {
    siteTitle: "kuru",
    nav: [
      { text: "Start", link: "/guide/installation", activeMatch: "/guide/" },
      { text: "Concepts", link: "/concepts/frameworks", activeMatch: "/concepts/" },
      { text: "Reference", link: "/reference/commands", activeMatch: "/reference/" },
    ],
    sidebar: [
      { items: [{ text: "Overview", link: "/" }] },
      {
        text: "Start here",
        items: [
          { text: "Installation & updates", link: "/guide/installation" },
          { text: "Authentication & models", link: "/guide/authentication" },
          { text: "Your first conversation", link: "/guide/first-conversation" },
        ],
      },
      {
        text: "How Kuru works",
        items: [
          { text: "Frameworks", link: "/concepts/frameworks" },
          { text: "Parts, relationships & memory", link: "/concepts/memory" },
          { text: "Sessions & dreaming", link: "/concepts/sessions" },
        ],
      },
      {
        text: "Reference",
        items: [
          { text: "Commands & shortcuts", link: "/reference/commands" },
          { text: "Configuration", link: "/reference/configuration" },
          { text: "Tools & permissions", link: "/reference/tools" },
          { text: "MCP servers", link: "/reference/mcp" },
          { text: "A2A peers", link: "/reference/a2a" },
        ],
      },
    ],
    search: { provider: "local" },
    outline: { level: [2, 3], label: "On this page" },
    socialLinks: [
      {
        icon: "github",
        link: "https://github.com/replygirl/kuru",
        ariaLabel: "Kuru source on GitHub (repository access required)",
      },
    ],
    footer: {
      message: "Persistent peers. Deliberate tools. Your terminal.",
      copyright: "Kuru · MIT licensed",
    },
    docFooter: { prev: "Previous", next: "Continue" },
  },
});
