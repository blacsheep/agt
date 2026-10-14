# {{ $params.title }}

::: info Chinese only
The architecture and design docs are written in Chinese. Read this page in Chinese:
<a :href="withBase(`/zh/architecture/${$params.page}`)">{{ $params.title }}</a>.
:::

The [Guide](/guide/what-is-agt) and [Modules](/modules/workspace) sections cover how to use agt in English.

<script setup>
import { withBase } from 'vitepress'
</script>
