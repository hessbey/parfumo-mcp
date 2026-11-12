# parfumo-mcp

[English](README.md) · [Français](README.fr.md) · **Deutsch**

<sub>Diese Übersetzung wurde nicht von einer Muttersprachlerin oder einem Muttersprachler
geprüft. Im Zweifel gilt die [englische Fassung](README.md).</sub>

[![ci](https://github.com/hessbey/parfumo-mcp/actions/workflows/ci.yml/badge.svg)](https://github.com/hessbey/parfumo-mcp/actions/workflows/ci.yml)
[![Lizenz: MIT](https://img.shields.io/badge/Lizenz-MIT-blue.svg)](LICENSE)
![rust 1.90+](https://img.shields.io/badge/rust-1.90%2B-orange.svg)

Ein in Rust geschriebener [Model-Context-Protocol](https://modelcontextprotocol.io)-Server,
der einem KI-Assistenten Zugriff auf [Parfumo](https://www.parfumo.com) gibt: auf die
Parfumdatenbank, deine Sammlung, deine getragenen Düfte, deine Bewertungen und Rezensionen sowie
auf die Community (Nachrichten, Forum, Blogs, Souk).

> **Inoffiziell.** Dieses Projekt steht in keiner Verbindung zu Parfumo und wird von Parfumo
> nicht unterstützt. Es liest dieselben Seiten wie ein Browser, mit einer Anfrage pro
> Sekunde. Automatisierter Zugriff kann trotzdem dazu führen, dass Parfumo das verknüpfte
> Konto sperrt – die Nutzung erfolgt auf eigene Gefahr.

## Was du bekommst

- **48 Tools**, alle mit dem Präfix `parfumo_`. Die 10 Datenbank-Tools und die öffentlichen
  Lesezugriffe (Forum, Blogs, Souk, Profile) funktionieren **ohne Parfumo-Konto**.
- **Zwei Programme, dieselben Tools:**

  | Programm | Transport | Für wen |
  | --- | --- | --- |
  | `parfumo-mcp` | stdio | Eine Person auf dem eigenen Rechner. Kein Port, kein OAuth, nichts zu hosten |
  | `parfumo-mcp-server` | Streamable HTTP | Mehrere Personen mit getrennten Konten, auf einem Server hinter HTTPS (Docker-Image) |

- **Standardmäßig vorsichtig.** Jedes schreibende Tool unterstützt `dry_run` und liest danach den
  tatsächlichen Stand zurück. Alles, was andere Mitglieder sehen können, bleibt ein
  Probelauf, bis du öffentliche Schreibzugriffe einschaltest – und ist dann pro Tool und
  Tag begrenzt.
- **Nachvollziehbar.** Jeder Tool-Aufruf wird lokal protokolliert; `parfumo-mcp audit --writes`
  zeigt, was tatsächlich in dein Konto geschrieben wurde.
- **Rücksichtsvoll gegenüber der Website.** Eine Anfrage pro Sekunde und Nutzer, zufällige
  Verzögerung, ein fester und ehrlicher `User-Agent`, 24 Stunden Cache für Seiten und eine
  Stunde für Suchen und Ranglisten. Kein Lösen von Captchas, kein Umgehen von Cloudflare.
- **Arbeitet mit `www.parfumo.com`, der englischen Website.** Die deutsche Website
  `www.parfumo.de` verwendet andere Adressen und Beschriftungen und wird nicht unterstützt.
- **Zugangsdaten bleiben außerhalb des Gesprächs.** Sie kommen aus Umgebungsvariablen (stdio)
  oder über die Webseite `/link` (HTTP), werden mit AES-256-GCM verschlüsselt gespeichert und
  erscheinen nie in Protokollen, Fehlermeldungen oder Tool-Ergebnissen.

## Schnellstart (lokal)

Lade das Archiv für dein System aus der [neuesten Version](https://github.com/hessbey/parfumo-mcp/releases/latest) herunter und entpacke es.
Es enthält das Programm `parfumo-mcp`, startbereit für Linux, macOS und Windows. Wie du es
selbst baust, steht unter [Lokale Nutzung](docs/LOCAL.md#install).

Zu Claude Code hinzufügen:

```sh
claude mcp add parfumo \
  -e PARFUMO_COOKIE='pf_auth=…' \
  -- /pfad/zu/parfumo-mcp
```

Oder in jedem Client, der über eine JSON-Datei konfiguriert wird (Claude Desktop und die
meisten anderen):

```json
{
  "mcpServers": {
    "parfumo": {
      "command": "/pfad/zu/parfumo-mcp",
      "env": { "PARFUMO_COOKIE": "pf_auth=…" }
    }
  }
}
```

`PARFUMO_COOKIE` ist das Cookie `pf_auth`, kopiert aus einem bei Parfumo angemeldeten
Browser. Stattdessen funktionieren auch `PARFUMO_USER` und `PARFUMO_PASSWORD` – oder gar nichts:
Ohne Konto funktionieren die öffentlichen Tools trotzdem. Details:
[Lokale Nutzung](docs/LOCAL.md).

Für den Betrieb mit mehreren Personen siehe [Hosting](docs/HOSTING.md): Der Server wird als
Docker-Image veröffentlicht, `ghcr.io/hessbey/parfumo-mcp-server`.

## Tools

| Bereich | Tools | Konto nötig |
| --- | --- | --- |
| Datenbank | Suche, Parfumseite, Rezensionen, Statements, ähnliche Parfums, Marke, Parfümeur, Duftnote, Ranglisten, Seitenabruf | Nein |
| Sammlung | auflisten, lesen, hinzufügen, verschieben, ändern, entfernen, eigene Listen, CSV-Export, Statistik | Ja |
| Getragene Düfte | eintragen, auflisten, löschen | Ja |
| Bewertungen und Rezensionen | bewerten, klassifizieren, Rezension oder Statement schreiben oder löschen, eigene auflisten | Ja |
| Community | Mitgliederprofile, Nachrichten, Benachrichtigungen, Abos, Pinnwand, Forum, Blogs, Souk, Communities | Lesen: meist nein. Schreiben: ja |
| Profil und Konto | Profil, Einstellungen, Sitzungsstatus | Ja |

Vollständige Liste: [Tool-Katalog](docs/TOOL_CATALOG.md).

## Ein Wort zu Texten

Parfumo verbietet KI-generierte Texte in Rezensionen, Statements, Forenbeiträgen und
Nachrichten und schränkt Konten ein oder löscht sie, wenn solche Texte veröffentlicht
werden. Die Tools, die Text senden, sagen das dem Assistenten: Er soll **deine** Worte
weitergeben, nicht an deiner Stelle schreiben.

## Dokumentation

Die Anleitungen sind auf Englisch, die ausführlichen Referenzen auf Französisch.

| Anleitung | Inhalt |
| --- | --- |
| [Lokale Nutzung](docs/LOCAL.md) | Das stdio-Programm: Konto, Client einrichten, lokale Dateien |
| [Hosting](docs/HOSTING.md) | Der HTTP-Server: Docker, Nutzer, OAuth, Parfumo-Konto verknüpfen |
| [Konfiguration](docs/CONFIGURATION.md) | Alle Umgebungsvariablen; Sperre, Tageslimits und Empfängerliste für öffentliche Schreibzugriffe |
| [Tool-Katalog](docs/TOOL_CATALOG.md) | Die 48 Tools, Konventionen, Fehlercodes |
| [Projektstand](docs/STATUS.md) | Was an der echten Website nachgewiesen ist, was nicht, was noch offen ist |
| [Alle Dokumente](docs/README.md) | Vollständiges Verzeichnis |

## Aufbau

```
crates/parfumo-client      Bibliothek: Zugriff auf die Website. Transport, Routen und
                           Selektoren (ein einziges Modul, src/site.rs), HTML-Auswertung,
                           Modelle
crates/parfumo-mcp-core    Bibliothek: die 48 Tools, Parfumo-Sitzung, Speicher, Audit-Protokoll
crates/parfumo-mcp         lokales Programm: die Tools über stdio
crates/parfumo-mcp-server  gehostetes Programm: die Tools über HTTP, mit OAuth, Nutzern, Canary
```

Parfumo wird serverseitig gerendert und hat keine öffentliche API, deshalb wertet der Client
HTML aus. Alle Routen und Selektoren liegen in einem einzigen Modul, die Parser werden gegen
aufgezeichnete Fixtures getestet, und ein täglicher, rein lesender Canary-Test auf dem
gehosteten Server meldet jede Seite, deren Struktur sich geändert hat.

## Entwicklung

```sh
cargo test --workspace     # ohne Netzwerk: aufgezeichnete Fixtures und lokale Fake-Server
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all --check
```

Die CI führt alle drei unter Linux aus, wiederholt die Tests unter macOS und Windows, baut
die beiden Release-Programme, baut danach das Docker-Image und prüft Größe, Start und
Speicherbedarf im Leerlauf. Ein Tag `vX.Y.Z` veröffentlicht eine Version: die Programme für
fünf Zielsysteme und das Docker-Image.

## Lizenz

[MIT](LICENSE)
