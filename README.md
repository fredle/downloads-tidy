# Downloads Tidy

Rule-based Downloads folder sorter with preview and one-click undo.

Downloads fills up with installers, zips, PDFs and images. Storage Sense can only delete by age and does not sort. Downloads Tidy sorts files into subfolders (or the Recycle Bin) using rules you control, shows exactly what it will do first, and lets you undo it.

## Features

- **Rules** by file type, age and size. Each rule either moves matching files into a subfolder or sends them to the Recycle Bin. The first matching rule wins; reorder them as you like. Six sensible defaults are included (an "old installers to Recycle Bin" rule ships switched off).
- **Dry-run preview**: the Preview tab lists every file, where it would go and why. Nothing changes until you click **Tidy now**.
- **Undo**: the last run is logged. Undo the whole run or restore single files. Name clashes never overwrite anything (`file (1).pdf`).
- **Scheduled runs** while the app is open (it can be minimised). Files modified in the last N minutes, files still downloading (`.crdownload`, `.part`, ...) and files open in another program are skipped.
- **Any folder**: point it at a relocated Downloads folder or any other folder. Only the top level is sorted; existing subfolders are left alone.

Keyboard: `F5` refresh preview, `Ctrl+Enter` tidy now, `Ctrl+Z` undo last run.

Files are never permanently deleted. Files sent to the Recycle Bin are restored from the Recycle Bin (the Undo tab has a button to open it).

## Install

- **Microsoft Store**: search for "Downloads Tidy".
- **GitHub Releases**: download the latest installer from the Releases page of this repository and run it. The app updates itself (toggle in the footer).

## Where data lives

`%LOCALAPPDATA%\DownloadsTidy\`

- `settings.json`: folder, rules, schedule, update preference
- `last_run.json`: the undo log of the last run

Uninstalling the app leaves these files; delete the folder to remove them.

## Privacy

No telemetry, no accounts, no admin rights. The only network call is the optional update check against this repository's GitHub Releases (about once a day, can be switched off in the footer). File names and contents never leave your PC.

## Build

```
cargo build --release
cargo test --release
```

Rust 2024, egui/eframe, Velopack for installation and updates.
