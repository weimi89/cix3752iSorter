<script setup>
import appIcon30 from '@images/app-icon-30.png'
import appIcon71 from '@images/app-icon-71.png'
import appIcon89 from '@images/app-icon-89.png'
</script>

<template>
  <img :src="appIcon30" :srcset="`${appIcon30} 1x, ${appIcon71} 2x, ${appIcon89} 3x`" width="30" height="30" alt="智配通" aria-hidden="true" draggable="false">
</template>
