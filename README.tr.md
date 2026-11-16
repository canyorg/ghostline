<p align="center"><a href="README.md">English</a> · <b>Türkçe</b> · <a href="README.ru.md">Русский</a></p>

```
 ██████╗ ██╗  ██╗ ██████╗ ███████╗████████╗██╗     ██╗███╗   ██╗███████╗
██╔════╝ ██║  ██║██╔═══██╗██╔════╝╚══██╔══╝██║     ██║████╗  ██║██╔════╝
██║  ███╗███████║██║   ██║███████╗   ██║   ██║     ██║██╔██╗ ██║█████╗
██║   ██║██╔══██║██║   ██║╚════██║   ██║   ██║     ██║██║╚██╗██║██╔══╝
╚██████╔╝██║  ██║╚██████╔╝███████║   ██║   ███████╗██║██║ ╚████║███████╗
 ╚═════╝ ╚═╝  ╚═╝ ╚═════╝ ╚══════╝   ╚═╝   ╚══════╝╚═╝╚═╝  ╚═══╝╚══════╝
```

# GhostLine

**Askeri seviye anonim sohbet: Signal Protocol + Tor onion taşıma, sıfır metadata.**

Hesap yok. Telefon numarası yok. E-posta yok. IP adresi yok. User agent yok. Log yok.
Bir oda numarası ve parola alırsın, takma adını seçersin, konuşursun, işin bitince odayı yakarsın.

> ⚠️ **Durum: pre-alpha, henüz denetlenmedi.** Kullanılan yapı taşları köklü ve güvenilir
> (Tor, Argon2id, X25519, Ed25519, XChaCha20-Poly1305), ancak bu uygulama henüz bağımsız
> olarak incelenmedi. Bir denetim yapılana kadar can güvenliğinin bağlı olduğu durumlarda
> tek başına güvenme. [Bilinen sınırlamalar](#bilinen-sınırlamalar) bölümüne bak.

<p align="center"><img src="docs/img/chat.png" alt="GhostLine masaüstü uygulaması" width="820"></p>

---

## Neden GhostLine? Zero trust (sıfır güven)

GhostLine, **hiç kimseye güvenmek zorunda kalmaman** için tasarlandı: hiçbir şirkete,
hiçbir uygulamaya, hiçbir sunucuya ve hiçbir GSM operatörüne.

- **Operatörler:** SMS ve operatör servisleri doğrudan dinlenebilir. Bunu operatörün kendisi,
  SS7 açıkları üzerinden başkaları ya da ağa erişimi olan herkes yapabilir. Üstelik telefon
  numarası doğrudan kimliğindir. GhostLine ikisini de kullanmaz.
- **WhatsApp kapalı kutu.** Signal Protocol kullandığını söylüyor, ancak uygulaması kapalı
  kaynak olduğu için bunu şirket dışından kimse kesin olarak doğrulayamaz. Ayrıca operatör
  metadata'yı (kim kiminle, ne zaman, ne sıklıkla konuşuyor) yine görür. Meta şirketine
  güvenmeyen çok sayıda insan var.
- **Zero trust** ilkesi "asla güvenme, her zaman doğrula" der. Özel iletişim için bunun
  anlamı açık kod, hesapsız ve numarasız kullanım, uçtan uca şifreleme ve kim olduğunu
  gizleyen bir ağ yoludur. Bu ilke gereği böyle bir uygulamanın kullanılması kaçınılmazdır.
- **Bize de güvenme.** Kod açık (AGPL-3.0). Oku, kendin derle, kendi relay'ini kur. Relay
  sadece şifreli veri tuttuğu için kötü niyetli bir relay bile işe yarar bir şey öğrenemez.

**Proje geliştirilebilir, fikirlerinizi bekliyoruz.** Bir [issue](../../issues) açabilir,
tartışma başlatabilir ya da pull request gönderebilirsiniz.

## Önerilen kurulum: GhostLine + Mullvad VPN

GhostLine, Tor sayesinde IP adresini relay'den zaten gizler. Gerçek gizlilik için Tor ve
GhostLine kullandığını **internet servis sağlayıcından (ISP)** da gizlemen gerekir. Bunun
için [Mullvad VPN](https://mullvad.net) öneriyoruz. E-posta veya kişisel bilgi istemez (sadece
hesap numarası), uygulamaları da açık kaynaklıdır. GhostLine'ı açmadan önce Mullvad'da
şunları aç:

| Ayar | Neden |
|---|---|
| **Kill switch** (her zaman açık) + **Lockdown mode** | VPN yeniden bağlanırken veya kapalıyken bile hiçbir trafik VPN dışına çıkmaz |
| **DAITA** | Dolgu ve sahte trafik ekler; yapay zeka tabanlı trafik analizi ne yaptığını tespit edemez |
| **Multihop (çoklu giriş) → Always** | Trafik bir Mullvad sunucusundan girer, başka birinden çıkar; hiçbir sunucu iki ucu birden görmez |

```
sen ──► Mullvad (2 atlama, DAITA) ──► Tor (3 atlama) ──► relay.onion (3 atlama)
        ISP sadece Mullvad'ı görür          relay IP görmez, sadece şifreli veri görür
```

Bu kombinasyon ideal sistemdir. VPN zorunlu değil, çünkü Tor tek başına IP adresini relay'den
gizliyor. Mullvad, ISP'ye ve yerel ağa karşı ek bir katman sağlar.

## Özellikler

| | |
|---|---|
| 🔐 **Uçtan uca şifreleme** | **Şu an:** Signal'in grup yöntemi *Sender Keys*. Her üyenin ileri yönde değişen bir anahtar zinciri var (forward secrecy), anahtarlar X25519 ile kişiye özel dağıtılıyor, her mesaj Ed25519 ile imzalanıyor ve biri ayrılınca herkes anahtar yeniliyor. XChaCha20-Poly1305 ve HKDF-SHA256 kullanılıyor. **Sırada:** tam `libsignal` (PQXDH + Double Ratchet). |
| 🧅 **Sadece Tor üzerinden** | **Şu an:** trafiği Tor'un üzerinden geçirir (`--tor`) ve uzaktaki bir relay'e Tor olmadan bağlanmayı reddeder. Relay bir `.onion` servisi, yani yol 6 atlamalı ve sunucu hiçbir IP görmez. **Sırada:** Arti ile uygulamaya gömülü Tor ve belirli aralıklarla değişen devreler. |
| 👻 **Sıfır metadata** | Hesap, IP, user agent ve kişi listesi yok. Relay, rastgele bir posta kutusu adresinde yalnızca dolgulu şifreli veriyi RAM'de tutar. Takma adlar sadece şifreli içeriğin içinde bulunur. |
| 🔢 **Sayısal oda ID'si** | Kripto cüzdanı veya Mullvad hesap numarası gibi: 16–23 hane, **uzunluğu her seferinde rastgele**. Damm kontrol hanesi, tek hanelik her yazım hatasını ve yan yana iki hanenin her yer değiştirmesini yakalar. |
| 🔑 **Oda parolası** | Kendi parolanı belirleyebilir ya da otomatik ürettirebilirsin. **"Şifre öner"** 16 karakterden uzun rastgele bir parola üretir (varsayılan ~123 bit). ID ve parola hiç değişmez. |
| 🏷️ **Odaya özel takma ad** | Odaya girerken seçilir, `takma_ad#parmakizi` olarak görünür. Unicode serbest; görünmez karakterler ve yazı yönünü değiştiren taklit karakterleri engellenir. |
| ✓✓ **İletim durumu** | ⏳ gönderiliyor · ✓ relay'de · ✓✓ herkese ulaştı · **cyan ✓✓** herkes okudu. Bilgilendirmeler şifreli gider, okundu bilgisi kapatılabilir. |
| 🚫 **Ekran görüntüsü koruması** | Windows'ta masaüstü uygulaması ekran görüntüsünde, ekran kaydında ve ekran paylaşımında görünmez. Linux'ta ve terminal istemcisinde bu mümkün değil, sınırlamalara bak. |
| 💥 **Odayı yak** | `/burn`: relay posta kutusunu anında siler (bir daha oluşturulamaz) ve tüm üyelerin istemcileri anahtarları siler. |
| 🟩 **Hacker arayüzü** | Matrix yağmurlu, mesaj balonlu, basılı tutunca silen **masaüstü uygulaması** (Linux + Windows). Türkçe, İngilizce ve Rusça. Yanında siyah üzerine yeşil **terminal istemcisi** de var ve ikisi birbiriyle konuşabiliyor. |
| 🧱 **Web motoru yok** | Masaüstü uygulaması saf Rust (egui): WebView, JavaScript ve HTML yok. Mesajlar her zaman düz metin olarak çizilir, diske hiçbir şey yazılmaz. |

## Hızlı başlangıç

[Rust](https://rustup.rs) gerekir. Linux'ta (Fedora, Debian, Arch…), Windows 10/11'de
(PowerShell, Windows Terminal) ve macOS'te çalışır.

```bash
git clone https://github.com/canyorg/ghostline && cd ghostline
cargo build --release
```

Çalıştırılabilir dosyalar `target/release/` klasörüne çıkar (Windows'ta sonlarında `.exe` olur):

| Dosya | Ne işe yarar |
|---|---|
| `ghostline-gui` | masaüstü uygulaması |
| `ghostline` | terminal istemcisi |
| `ghostline-relay` | relay sunucusu |

### Masaüstü uygulaması

```bash
./target/release/ghostline-relay &        # yerel test için; gerçek kullanımda .onion relay
./target/release/ghostline-gui
```

**CHAT OLUŞTUR** → takma ad → parola (otomatik üretilir ya da kendin belirlersin; **Şifre öner**
yenisini verir) → oda ID'si ve parola kopyalama butonlarıyla gösterilir → **Odaya gir**.
**ODAYA KATIL** → oda ID'si (yazım hataları yazarken yakalanır) → parola → takma ad.

Oda içinde: 👥 üyeler, oda bilgisi (ID ve parolayı tekrar paylaşmak için), **Çık** ve
**🔥 ODAYI SİL**: 1,5 saniye basılı tutunca oda herkes için anında yok olur. Relay ve Tor ayarları
başlangıç ekranındaki **Ağ** bölümünde. Dil sistem dilini takip eder (TR/EN/RU), aynı ekrandan
değiştirilebilir.

<p align="center"><img src="docs/img/home.png" width="400"> <img src="docs/img/created.png" width="400"></p>

### Terminal istemcisi

3 terminalde yerel deneme:

```bash
# terminal 1 — relay (sadece RAM'de çalışır, hiçbir şey loglamaz)
./target/release/ghostline-relay

# terminal 2 — oda oluştur (parola sorulunca Enter'a basarsan otomatik üretilir)
./target/release/ghostline create --nick "Zero Cool"

# terminal 3 — terminal 2'de görünen ID ve parola ile katıl
./target/release/ghostline join 1076 7380 0491 6080 894 --nick "Acid Burn"
```

Windows'ta aynı argümanlarla `.\target\release\ghostline.exe` kullan.

Oda içindeki komutlar: `/help` · `/who` · `/info` · `/quit` (Ctrl+C) · `/burn`

### Tor üzerinden (gerçek kullanım)

1. **Relay'i onion servisi olarak çalıştır** (herhangi bir Linux makinede):

   ```bash
   sudo dnf install tor          # Debian/Ubuntu: sudo apt install tor
   sudo tee -a /etc/tor/torrc <<'TORRC'
   HiddenServiceDir /var/lib/tor/ghostline/
   HiddenServicePort 80 127.0.0.1:7878
   TORRC
   sudo systemctl restart tor
   sudo cat /var/lib/tor/ghostline/hostname     # → abcd…xyz.onion
   ./target/release/ghostline-relay              # 127.0.0.1'de kalır
   ```

2. **İstemcilerde** Tor'un çalışıyor olması gerekir. Linux'ta: `sudo dnf install tor && sudo systemctl start tor`
   (port 9050). Windows'ta veya Linux'ta **Tor Browser**'ı açman da yeterli (port 9150).

   ```bash
   ghostline create --relay http://abcd…xyz.onion
   ghostline join <ID> --relay http://abcd…xyz.onion --tor-port 9150   # Tor Browser ile
   ```

   İpucu: `export GHOSTLINE_RELAY=http://abcd…xyz.onion` (PowerShell: `$env:GHOSTLINE_RELAY="…"`).

## Oda nasıl çalışır

```
OLUŞTUR                                  KATIL
takma ad: Zero Cool                      oda id  : 3414 7650 6754 3639 1733 042
oda id  : 3414 7650 6754 3639 1733 042   parola  : ********************
parola  : [otomatik]  [şifre öner]       takma ad: Acid Burn
        └──► Argon2id(parola, id) ──► oda anahtarı · posta kutusu id · silme anahtarı
```

ID tek başına işe yaramaz: posta kutusu adresi **paroladan** türetildiği için parola olmadan
oda bulunamaz bile. Yanlış parola ile var olmayan oda birbirinden ayırt edilemez.

Ayrıntılar: [`docs/ARCHITECTURE.md`](docs/ARCHITECTURE.md) · [`docs/THREAT_MODEL.md`](docs/THREAT_MODEL.md)

## Bilinen sınırlamalar

Bir güvenlik aracı neyi **yapamadığını** açıkça söylemeli:

1. **Silme sadece değiştirilmemiş istemcilerde garanti.** "Odayı yak" relay'deki veriyi ve
   dürüst istemcilerdeki her şeyi siler. Değiştirilmiş bir istemci veya mesajları önceden
   kopyalamış biri bunları saklayabilir. Hiçbir mesajlaşma uygulaması bunu engelleyemez.
2. **Ekran görüntüsü koruması sınırlı.** Terminal istemcisinde koruma yok. Windows masaüstü
   uygulaması pencereyi ekran yakalamadan gizler, ancak sanal makine veya capture kartı ile
   aşılabilir. Linux'ta bu mümkün değil (X11'de engellenemez, Wayland'de compositor'a bağlı).
   Ekranın telefonla fotoğrafının çekilmesini hiçbir yazılım engelleyemez.
3. **Ele geçirilmiş bir cihaz her şeyi boşa çıkarır.** Zararlı yazılım veya kilidi açık
   bilgisayarının başındaki biri, senin gördüğün her şeyi görür.
4. **Oda parolası ortak bir sırdır.** ID'yi ve parolayı bilen herkes odaya girebilir ve odayı
   yakabilir. Forward secrecy sayesinde geçmiş mesajları okuyamaz.
5. **Zayıf parolalar çevrimdışı kırılabilir.** Bunun için relay verisine erişim gerekir.
   Argon2id (128 MiB) bunu ciddi şekilde yavaşlatır, yine de üretilen parolaları kullan.
6. **Tor, küresel bir gözlemciye karşı yetmez.** Ağın iki ucunu birden izleyen biri zamanlamayı
   eşleştirebilir. Bu yüzden okundu bilgisi kapatılabiliyor; Mullvad DAITA da burada yardımcı olur.
7. **Relay işletmecisi şunları görür:** posta kutusu ID'leri, dolgulu boyutlar ve verinin geliş
   zamanları. IP, takma ad, oda ID'si, parola ve içerik **görmez**.
8. **Tor'un engellendiği ülkelerde** köprü (bridge) kullanman ya da Tor'un önüne Mullvad koyman gerekir.
9. **Bellek temizliği en iyi çaba esaslıdır.** Anahtarlar kullanıldıktan sonra silinir, ancak
   swap, hazırda bekletme dosyası ve çökme dökümlerinde kalıntı olabilir. Tam disk şifreleme kullan.
10. **Grup şifrelemesi ara çözüm.** v0'daki Sender Keys forward secrecy sağlar ama
    post-compromise security sağlamaz. libsignal yol haritasında.
11. **Parmak izleri kısa.** `#1a2b3c` 24 bit: birinin yerine başkasının geçtiğini fark etmeye
    yeter, kimliği kanıtlamaya yetmez.
12. **Henüz denetlenmedi.**

## Yol haritası

- [x] **M1** Oda ID'leri (Damm), parola üretici, takma ad doğrulama, Argon2id/HKDF
- [x] **M2a** Uçtan uca grup mesajlaşması (Sender Keys), imzalar, dolgu, ✓/✓✓, ayrılışta anahtar yenileme
- [x] **M3** Relay: sadece RAM, log yok, TTL, anında silme ve kalıcı silinme kaydı (tombstone)
- [x] **CLI** Linux ve Windows terminal istemcisi, SOCKS ile Tor
- [x] **M6a** Odayı yakma
- [x] **M5** Masaüstü uygulaması (Linux + Windows): matrix arayüzü, balonlar, ✔/✔✔, basılı tutarak silme, Windows'ta ekran yakalama engeli, TR/EN/RU
- [ ] **M2b** `libsignal` (PQXDH + Double Ratchet)
- [ ] **M4** Gömülü Tor (Arti): devre değişimi, akış izolasyonu, köprüler
- [ ] **M6b** Panik silme, isteğe bağlı şifreli yerel geçmiş
- [ ] **M7** Android
- [ ] **M8** Bağımsız güvenlik denetimi

## Katkı

Fikirler, hata bildirimleri, çeviriler, kod incelemeleri ve pull request'ler memnuniyetle
karşılanır. Güvenlik açıklarını lütfen [SECURITY.md](SECURITY.md)'de anlatıldığı gibi gizli bildir.

```bash
cargo test                                   # birim testleri + protokol simülasyonu
cargo clippy --all-targets -- -D warnings
```

```
crates/core    protokol çekirdeği: ID, KDF, kripto, mesaj formatı, oturum
crates/relay   relay sunucusu (sadece RAM, log yok)
crates/client  ortak katman: relay bağlantısı (Tor/SOCKS), oluştur/katıl, oda sürücüsü
crates/cli     terminal istemcisi `ghostline`
crates/gui     masaüstü uygulaması `ghostline-gui` (egui)
docs/          mimari ve tehdit modeli
```

## Lisans

AGPL-3.0-or-later.
