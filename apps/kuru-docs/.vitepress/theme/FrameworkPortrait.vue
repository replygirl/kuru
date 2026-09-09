<script setup lang="ts">
import { computed, ref } from "vue";
import { withBase } from "vitepress";

const frameworks = [
  {
    id: "ifs",
    title: "IFS",
    subtitle: "Parts in relationship",
    description:
      "Preparation, recovery, and overlooked needs each have a voice. Self participates as a peer.",
    members: "Self · Planner · Guardian · Responder · Restorer · Witness · Hope",
    art: String.raw`           . . . . . . .
       . :               : .
     .     +           +     .
   .          . . . .          .
  :     .   .         .   .     :
  .   +    :     +     :    +   .
  :     .   .         .   .     :
   .          . . . .          .
     .     +           +     .
       . :               : .
           . . . . . . .`,
  },
  {
    id: "polyvagal",
    title: "Polyvagal",
    subtitle: "Connection, action, conservation",
    description:
      "Three peers bring engagement, purposeful action, and the value of slowing down into the conversation.",
    members: "Connection · Mobilization · Conservation",
    art: String.raw`        . . .                 . .
      .       .             .
  - +           . . . . . .

  . . .               . . . . .
        .           .           .
          . . + . .

              . . . . . .
            .             .
  . . . . .                 + -`,
  },
  {
    id: "freudian",
    title: "Freudian",
    subtitle: "Impulse, reality, standards",
    description:
      "Desire opens possibilities, Reality checks constraints, and Standards questions what good work means.",
    members: "Desire · Reality · Standards",
    art: String.raw`                 +
                / \
               /   \
              .     .
             / . : . \
            /  : . :  \
           . . . : . . .
          /     . .     \
         /       :       \
        + - - .     . - - +
             . . . . .`,
  },
  {
    id: "jungian",
    title: "Jungian",
    subtitle: "Patterns, shadow, shared memory",
    description:
      "Five peers attend to continuity, expression, neglected assumptions, contrasting perspectives, and project knowledge.",
    members: "Continuity · Interface · Shadow · Bridge · Collective",
    art: String.raw`              . + .
          . :       : .
      . .     . . .     . .
    .     . .       . .     .
   +      :     .     :      +
    .   . :   .   .   : .   .
      :   .     .     .   :
    .   .   . .   . .   .   .
   +      .           .      +
     . .    . : . : .    . .
              . . .`,
  },
];
const selected = ref(0);
const current = computed(() => frameworks[selected.value]);
</script>

<template>
  <section
    class="framework-portrait"
    :data-framework="current.id"
    aria-label="Explore Kuru frameworks"
  >
    <div class="portrait-topline"><span>THE PEER POOL</span><span>04 FRAMEWORKS</span></div>
    <div class="portrait-art" aria-hidden="true">
      <pre>{{ current.art }}</pre>
    </div>
    <div class="portrait-options" role="group" aria-label="Choose a framework preview">
      <button
        v-for="(framework, index) in frameworks"
        :key="framework.id"
        type="button"
        :aria-pressed="selected === index"
        @click="selected = index"
      >
        {{ framework.title }}
      </button>
    </div>
    <div class="portrait-copy" aria-live="polite" aria-atomic="true">
      <h2>{{ current.subtitle }}</h2>
      <p>{{ current.description }}</p>
      <p class="portrait-members">{{ current.members }}</p>
    </div>
    <a class="portrait-link" :href="withBase(`/concepts/frameworks#${current.id}`)"
      >Meet the {{ current.title }} peers <span aria-hidden="true">↗</span></a
    >
  </section>
</template>
