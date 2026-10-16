# ERPCons changes

This branch (`erpcons`) is the ERPCons Văn phòng fork of [DeckCraft](https://github.com/storytold/deckcraft), based on DeckCraft by the ArtCraft team, licensed MIT OR Apache-2.0 (see `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). Changes from upstream `main` (Apache-2.0 §4b):

- Removed the ArtCraft logos and marks (`docs/brand/`) and every link to the ArtCraft Discord and getartcraft.com, as the ArtCraft brand terms require for forks: About dialog, Backstage Home and Account, the ribbon's "Learn More" group, `app.links`, the CLI and the MCP server info, README.
- About now reads "ERPCons Văn phòng — dựa trên DeckCraft của nhóm ArtCraft. MIT OR Apache-2.0."; source links point at this fork.
- On a host with no save dialog (the web build), Save and Save As download a `.pptx` for every presentation, including new ones and `.deckcraft` files, and the download marks the presentation saved. Desktop builds still default to `.deckcraft`.
- Input methods that rewrite letters already typed (Vietnamese Telex/VNI in a browser, phone keyboard corrections) no longer garble slide text ("aá" for "á"): egui's `DeleteSurrounding` IME event runs the new `text.deleteSurrounding` command (`Session::delete_surrounding`), which deletes around the insertion point or composition within the paragraph; the replacement that follows (text, or a preedit and commit) is one undo step with the deletion.
- The web build asks before the tab is closed, reloaded or left while a presentation has unsaved changes or a save is still in the save picker (`beforeunload`).
- On browsers with a save picker (`showSaveFilePicker`, Chromium), web Save and Save As write through it (`Services::save_file`) and mark the presentation saved only once the file is written, as it was when written: cancelling keeps the changes unsaved, a failure says why in the status bar, and edits made while the picker was open stay unsaved. Other browsers keep the download, which marks it saved at once.
