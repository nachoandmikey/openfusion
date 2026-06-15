# Distribution notes

OpenFusion is a Rust CLI. It does **not** need Vercel or a hosted backend for the core product.

## Best launch path

1. **Cargo/native install now**
   ```bash
   cargo install --git https://github.com/nachoiacovino/openfusion
   ```
   This is the canonical install path today. It requires Rust/Cargo but avoids npm tokens and hosted infrastructure.

2. **GitHub Releases with native binaries**
   Use `cargo-dist` or a release workflow to publish macOS/Linux/Windows binaries. Users download one file; no Rust toolchain needed.

3. **Homebrew tap**
   Best Mac developer UX after the name stabilizes:
   ```bash
   brew install nachoiacovino/tap/openfusion
   ```

4. **crates.io**
   Good for Rust users once the package name is reserved:
   ```bash
   cargo install openfusion
   ```

5. **npm wrapper later, optional**
   Useful because many agent/dev-tool users have Node installed:
   ```bash
   npm i -g openfusion
   ```
   The npm package should be a thin wrapper that downloads the right native binary from GitHub Releases. This needs an npm token later, but it is not needed for the initial Cargo-first launch.

## Do we need hosting?

Not for the CLI. OpenFusion runs locally and reads the user's own env vars / local CLIs / local model servers.

Potential hosted pieces later:

- documentation site (GitHub Pages, Vercel, or Docusaurus)
- hosted demo page
- remote strategy registry/templates
- telemetry-free update metadata

Avoid a hosted inference service for now; it would introduce key custody, billing, and trust problems. The product is strongest as local-first.

## Recommendation

For launch:

- GitHub public repo
- `cargo install --git https://github.com/nachoiacovino/openfusion` in README
- companion skill install via `openfusion skills install`
- GitHub release binaries if time permits
- npm wrapper only after native binary release and npm token
