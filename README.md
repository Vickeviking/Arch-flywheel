# Arch-flywheel 🦀

En GTA-stil radial meny (pie/flywheel) för **Hyprland**, byggd i Rust med
GTK4 + layer-shell. Snabb väg till dokumentation: Rust/C-manualer, crate-docs,
nvim-hjälp, Arch Wiki — data-drivet via `menu.toml`.

## Kontroller

Tryck **SUPER+SPACE** och **släpp** (håll *inte* — då blir varje tangent en
SUPER-chord som Hyprlands globala bindningar fångar).

| Handling | Mus | Tangent |
|---|---|---|
| Markera | för musen i riktning | `J`/`K` eller piltangenter |
| Gå in i kategori / öppna blad | vänsterklick | `SPACE` / `Enter` / `L` |
| Backa en nivå | högerklick | `H` / `Backspace` |
| Stäng | klick i mitten | `Esc` / `q` |

## Meny

Första körningen skrivs `~/.config/arch-flywheel/menu.toml`. Redigera fritt —
varje post har `label` och antingen `items` (undermeny), `url` (öppnas med
`xdg-open`) eller `cmd` (körs via `sh -c`). Filen läses om varje gång hjulet
öppnas.

## Bygga

```sh
cargo build --release
install -m755 target/release/arch-flywheel ~/.local/bin/arch-flywheel
```

Kräver `gtk4` och `gtk4-layer-shell` (Arch: `pacman -S gtk4 gtk4-layer-shell`).

## Hyprland-bind

```
bind = SUPER, SPACE, exec, arch-flywheel
```
