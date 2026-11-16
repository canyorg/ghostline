**🇬🇧 English** · **🇹🇷 Türkçe** · **🇷🇺 Русский** — see the [README](https://github.com/canyorg/ghostline#readme)

> ⚠️ Pre-alpha, not audited. / Pre-alpha, henüz denetlenmedi. / Pre-alpha, аудит не проводился.

### Downloads / İndirme / Скачать

| File | What |
|---|---|
| `ghostline-gui-x86_64-pc-windows-msvc.exe` | **Windows desktop app** / masaüstü uygulaması / десктоп-приложение |
| `ghostline-x86_64-pc-windows-msvc.exe` | Windows terminal client (PowerShell) |
| `ghostline-relay-x86_64-pc-windows-msvc.exe` | relay server |
| `ghostline-gui-x86_64-unknown-linux-gnu` | Linux desktop app (`chmod +x`) |
| `ghostline-x86_64-unknown-linux-gnu` | Linux terminal client |
| `ghostline-relay-x86_64-unknown-linux-gnu` | Linux relay |
| `SHA256SUMS-*.txt` | checksums — verify before running |

**Windows SmartScreen:** the .exe is not code-signed, so Windows may say *"Windows protected your PC"* → **More info → Run anyway**. Verify the SHA-256 first:
`Get-FileHash .\ghostline-gui-x86_64-pc-windows-msvc.exe -Algorithm SHA256`

**Tor:** for an `.onion` relay keep **Tor Browser** open and set the Tor port to **9150** under *Network / Ağ / Сеть*.

**Max OPSEC:** prefer the Linux build inside a Fedora/Kali VM behind Mullvad (see README).
