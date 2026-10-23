# Octos SDK (frozen)

This directory is a verbatim copy of the upstream Octos mod-facing JavaScript API
(`src/js/`), plus the original rollup config. It is the **compatibility baseline**:
do not "modernize" or rename fields here during M3. The Rust host must satisfy this
library's wire behavior.

Build (from this directory):

```bash
npm install
npm run build-api
```

The UMD global remains `octos`; the output remains `octos.min.js`.

Source of truth: `/home/lzt/codehub/rust/octos/octos/src/js` (read-only).
