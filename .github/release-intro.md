Desktop WhatsApp archive and client (Rust + Tauri v2 + Svelte 5).

**Downloads**
- Linux: `.deb` (Ubuntu/Debian) or `.AppImage` (any distro)
- Windows: `.msi` or `-setup.exe`
- `SHA256SUMS.txt`: checksums of every download (`sha256sum -c SHA256SUMS.txt`)

**WhatsApp bridge and MCP server included.** The app ships the
[wa-bridge](https://github.com/rishindra2005/wa-bridge) service and starts it when no bridge is running.
Link your phone from **System → Bridge** (WhatsApp → Linked devices → Link a device). If you already run
your own bridge, set **Settings → WhatsApp bridge** to *External*. The MCP server (on by default, needs
[uv](https://docs.astral.sh/uv/)) is at `http://127.0.0.1:8771/mcp`; turn it off in **Settings → MCP server**.

**Windows:** the Windows build compiles and passes lint in CI but has not been tried on a real Windows PC
yet. Phone sync (MTP) and withdrawing notifications after you read a chat are Linux-only.
