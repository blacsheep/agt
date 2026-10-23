<script setup lang="ts">
import { useData, withBase } from 'vitepress'

// Cards come from the home page's `showcase` frontmatter, one list per locale.
const { frontmatter } = useData()
</script>

<template>
  <section v-if="frontmatter.showcase" class="showcase">
    <div class="container">
      <h2 class="title">{{ frontmatter.showcase.title }}</h2>
      <div class="cards">
        <a v-for="item in frontmatter.showcase.items" :key="item.link" class="card" :href="withBase(item.link)">
          <img :src="withBase(item.image)" :alt="item.alt" loading="lazy">
          <div class="body">
            <h3>{{ item.title }}</h3>
            <p>{{ item.details }}</p>
          </div>
        </a>
      </div>
    </div>
  </section>
</template>

<style scoped>
.showcase {
  padding: 0 24px 48px;
}
@media (min-width: 640px) {
  .showcase { padding: 0 48px 48px; }
}
@media (min-width: 960px) {
  .showcase { padding: 0 64px 56px; }
}
.container {
  margin: 0 auto;
  max-width: 1152px;
}
.title {
  margin-bottom: 16px;
  font-size: 20px;
  font-weight: 600;
}
.cards {
  display: grid;
  gap: 16px;
  grid-template-columns: 1fr;
}
@media (min-width: 768px) {
  .cards { grid-template-columns: repeat(3, 1fr); }
}
.card {
  display: flex;
  flex-direction: column;
  overflow: hidden;
  border: 1px solid var(--vp-c-bg-soft);
  border-radius: 12px;
  background-color: var(--vp-c-bg-soft);
  color: inherit;
  text-decoration: none;
  transition: border-color 0.25s;
}
.card:hover {
  border-color: var(--vp-c-brand-1);
}
.card img {
  display: block;
  width: 100%;
  aspect-ratio: 16 / 10;
  object-fit: cover;
  object-position: top;
  background-color: var(--vp-c-bg-alt);
}
.body {
  padding: 16px 20px 20px;
}
.body h3 {
  font-size: 16px;
  font-weight: 600;
  line-height: 24px;
}
.body p {
  padding-top: 8px;
  font-size: 14px;
  line-height: 24px;
  color: var(--vp-c-text-2);
}
</style>
