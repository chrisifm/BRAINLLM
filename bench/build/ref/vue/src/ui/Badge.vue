<script setup>
import './Badge.css'
defineProps({tone:{default:'neutral'},text:String})
</script>
<template><span class="badge" :data-tone="tone">{{text}}</span></template>
