# ToonSettings

A simple and lightweight settings copier for the game Eve Online which allows players to quickly copy character settings (overview, window layouts, shortcuts, etc.) from one character to others. Tested on Ubuntu 24.04.3 LTS.

## Requirements

* Rust toolchain (for compilation)
* Eve Online installed via Steam (Proton)

## Installation

1. Clone the repository and navigate to the project directory.
2. Build the binary:

```
cargo build --release
```

3. The compiled binary will be located at `target/release/ToonSettings`.
4. Copy it to a location in your PATH or create a desktop entry to run it as an application.

## Usage

1. Launch ToonSettings as an application.
2. The program will automatically scan for Eve Online character settings files.
3. You will see each character's settings file listed with their character name (fetched from CCP's ESI API).
4. Select one character under "Copy From" (the source).
5. Select one or more characters under "Copy To" (the destinations).
6. Click "Copy Settings" to copy the settings from the source to all selected destinations.

The main window displays all detected character settings files and shows character names alongside the file IDs. A popup will confirm whether the copy operation succeeded or failed.

## Settings Location

ToonSettings scans for Eve Online settings files in the default Steam/Proton location:

```
~/.steam/steam/steamapps/compatdata/8500/pfx/drive_c/users/steamuser/AppData/Local/CCP/EVE/
```

The path can be manually changed in the application if your Eve installation is in a different location.

## What Gets Copied

Eve Online stores character-specific settings in `core_char_[ID].dat` files. These files contain:

* Overview settings and profiles
* Window positions and layouts
* Keyboard shortcuts
* UI preferences
* Chat channel settings
* And other character-specific configurations

## Notes

* ToonSettings only works with character settings files (`core_char_*.dat`), not account-level settings (`core_user_*.dat`).
* Character names are fetched from CCP's official ESI API (esi.evetech.net).
* The copy operation overwrites the destination file entirely with the source file's contents.
* It is recommended to back up your settings files before using this tool.
* Eve Online should be closed when copying settings to avoid conflicts.
