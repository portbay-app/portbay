import { h } from "vue";
import DefaultTheme from "vitepress/theme";
import type { Theme } from "vitepress";
import ThemeImage from "./ThemeImage.vue";
import FeedbackWidget from "./FeedbackWidget.vue";
import "./custom.css";

export default {
  extends: DefaultTheme,
  // "Was this helpful?" widget — floats in once the reader has scrolled 70%
  // of a docs page (doc layout only, so never on the home landing page).
  Layout: () =>
    h(DefaultTheme.Layout, null, {
      "doc-after": () => h(FeedbackWidget),
    }),
  enhanceApp({ app }) {
    app.component("ThemeImage", ThemeImage);
  },
} satisfies Theme;
