import { h } from "vue";
import DefaultTheme from "vitepress/theme";
import type { Theme } from "vitepress";
import FrameworkPortrait from "./FrameworkPortrait.vue";
import "./custom.css";

export default {
  extends: DefaultTheme,
  Layout: () =>
    h(DefaultTheme.Layout, null, {
      "home-hero-image": () => h(FrameworkPortrait),
    }),
  enhanceApp({ app }) {
    app.component("FrameworkPortrait", FrameworkPortrait);
  },
} satisfies Theme;
