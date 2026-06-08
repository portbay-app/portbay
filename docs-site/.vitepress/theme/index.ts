import { h } from "vue";
import DefaultTheme from "vitepress/theme";
import type { Theme } from "vitepress";
import ThemeImage from "./ThemeImage.vue";
import FeedbackWidget from "./FeedbackWidget.vue";
import "./custom.css";

export default {
  extends: DefaultTheme,
  // "Was this helpful?" widget at the bottom of every docs page.
  Layout: () =>
    h(DefaultTheme.Layout, null, {
      "doc-after": () => h(FeedbackWidget),
    }),
  enhanceApp({ app }) {
    app.component("ThemeImage", ThemeImage);
  },
} satisfies Theme;
