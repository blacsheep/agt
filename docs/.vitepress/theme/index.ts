import DefaultTheme from 'vitepress/theme'
import { h } from 'vue'
import Showcase from './Showcase.vue'

// The default theme, with the home page's showcase cards above the features.
export default {
  extends: DefaultTheme,
  Layout: () => h(DefaultTheme.Layout, null, { 'home-features-before': () => h(Showcase) }),
}
