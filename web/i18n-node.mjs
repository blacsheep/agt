// The server shares public/commands.js with the page but never shows its
// labels, so in Node $t returns the Chinese source text as is.
globalThis.$t ??= text => text;
