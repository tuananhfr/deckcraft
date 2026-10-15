# ERPCons changes

This branch (`erpcons`) is the ERPCons Văn phòng fork of [DeckCraft](https://github.com/storytold/deckcraft), based on DeckCraft by the ArtCraft team, licensed MIT OR Apache-2.0 (see `LICENSE-MIT`, `LICENSE-APACHE`, `NOTICE`). Changes from upstream `main` (Apache-2.0 §4b):

- Removed the ArtCraft logos and marks (`docs/brand/`) and every link to the ArtCraft Discord and getartcraft.com, as the ArtCraft brand terms require for forks: About dialog, Backstage Home and Account, the ribbon's "Learn More" group, `app.links`, the CLI and the MCP server info, README.
- About now reads "ERPCons Văn phòng — dựa trên DeckCraft của nhóm ArtCraft. MIT OR Apache-2.0."; source links point at this fork.
- On a host with no save dialog (the web build), Save and Save As download a `.pptx` for every presentation, including new ones and `.deckcraft` files, and the download marks the presentation saved. Desktop builds still default to `.deckcraft`.
