# ambilight — leichtgewichtiges WLED-Ambilight für Linux (Wayland/KDE)

Ein Rust-Programm, das den Bildschirm aufnimmt, frei platzierbare Bereiche
(„Zonen") abtastet und die Durchschnittsfarbe dieser Bereiche als DDP-Pakete an
WLED schickt. Alles läuft über normalisierte Koordinaten — Auflösung,
Skalierung und Moduswechsel sind damit egal.

## Bauen & starten

```sh
cd ~/projects/wled-ambilight
cargo build --release
./target/release/ambilight            # GUI
./target/release/ambilight --headless # ohne Fenster, für Autostart
```

Geräte werden gesucht (mDNS + Subnetz-Sweep), nicht eingetippt.

Voraussetzungen zur Laufzeit: `gst-launch-1.0` mit `pipewiresrc`
(Fedora/Bazzite: `gstreamer1-plugins-bad-free`), ein laufendes
`xdg-desktop-portal` + Backend, Netzwerkzugriff auf die WLED-Geräte.

Beim ersten Start fragt das Portal einmalig, welcher Monitor aufgenommen werden
soll. Der Restore-Token wird in `~/.config/wled-ambilight/config.json`
gespeichert, danach startet die Aufnahme ohne Dialog.

## Konzept

- **Device** = ein WLED-Knoten mit *n* LEDs (Anzahl wird per `/json/info`
  automatisch erkannt).
- **Zone** = ein Rechteck auf dem Bildschirm + eine Anzahl LEDs. Jede Zone
  belegt einen Abschnitt des Geräts; die Reihenfolge in der Zonenliste ist die
  Reihenfolge auf dem Streifen (LED 0…n). Die Zonenliste zeigt den Bereich an
  (`LEDs 0-29`).
- Für jede LED wird der Bildbereich der Zone in gleich breite/höhe Zellen
  geteilt und deren Durchschnittsfarbe gesampelt — dadurch funktioniert jede
  LED-Anzahl unabhängig von der Pixelzahl.
- Ein Streifen rund um den Bildschirm = 4 Zonen (oben, rechts, unten, links) auf
  **einem** Device. Ein einzelner Balken = 1 Zone (die anderen löschen).

## Bedienung

- Zonen mit der Maus verschieben, an Ecken/Kanten größer/kleiner ziehen
  (Snap an Bildschirmkante, Mitte und andere Zonen ist standardmäßig an).
- Pfeiltasten verschieben die ausgewählte Zone (Shift = größere Schritte).
- Presets `Top / Bottom / Left / Right / Full / Zone` legen neue Zonen an.
- Pro Gerät: **„Kette läuft rückwärts"** — ein Schalter für die Einbaurichtung.
  Er spiegelt das komplette Bild und dreht bei geteilten Streifen auch die
  Reihenfolge der Zonen. `BGR` nur für Klone mit vertauschten Kanälen.
- Neben dem Bild liegt für jede Zone ein **LED-Lineal**: ein Kästchen pro LED in
  der Farbe, die gerade gesendet wird. Unten zeigt die Ausgabe-Leiste dasselbe
  pro Gerät.
- **Bildrate**: global (24–144 fps) und pro Gerät ein eigenes Limit
  (`max. FPS`, Standardwerte im Dropdown). Die Pipeline läuft auf einem
  Vielfachen der Zielrate, damit die Limits exakt greifen.
- **Glättung**: Median über drei Messungen, dann exponentielle Glättung
  (Standard 180 ms) und eine Begrenzung der Änderungsrate.

## Ausgabe

- **DDP** (UDP 4048) ist der Default: beliebig viele LEDs, 480 LEDs pro Paket.
- **DRGB** (UDP 21324) als Fallback für alte WLED-Versionen (max. ~490 LEDs).

## Konfiguration

`~/.config/wled-ambilight/config.json`, wird automatisch gespeichert.
Alle Koordinaten sind 0…1 relativ zum aufgenommenen Bildschirm.
