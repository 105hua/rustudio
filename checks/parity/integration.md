# Integration fixes (2026-11-08)

## Pages: one owner (src/domains/pages.rs)
- Pages:GetCurrentPage / AdvanceToNextPageAsync / IsFinished live only in
  pages.rs. Producers supply a source and nothing else:
  `__rs_pages_new(class, next)` with `next() -> (page, finished)` (owns its
  cursor, may yield, called for the first page before returning), or
  `__rs_pages_list(class, pages, onAdvance?)` / `pages::register_list` for
  pages already fetched.
- Behaviour: fresh deep copy per GetCurrentPage; IsFinished on the last
  page; advancing a finished Pages errors "No pages to advance to" (string in
  Studio) without yielding; advancing always yields; a failed
  fetch leaves page and IsFinished unchanged.
- Producers moved: DataStore*Pages, MemoryStoreHashMapPages, local-mode
  GetSortedAsync (was a plain table, now a paged DataStorePages), FriendPages,
  Inventory/Outfit/CatalogPages, AnimationClipProvider InventoryPages,
  StandardPages / RecommendationPages, AudioPages.
- Not checked against a live Studio (offline Studio MCP was down): whether a second
  AdvanceToNextPageAsync while one is in flight errors in Studio. Today both
  run the source.

## Play waits for the opened place's ids
- Play (and harness --play) during a Roblox open / Team Create question /
  live join is held (cloud.play_when_loaded) and starts once the place is the
  document, so game.PlaceId / GameId / CreatorId are set before scripts.
  Verified: --open-roblox 95206881 --play → 95206881 / 28220420 / 998796.

## Dialog speech
- MainBotChatScript2 already speaks through Chat:Chat (ChatLocal → Chat:Chat
  stand-in). The gap was drawing: Chat:Chat bubbles only drew for a
  TextChatService place with CoreGui Chat on. They now draw for any
  ChatVersion and with CoreGui Chat off (ExpChat's bubbles run regardless;
  assumes FFlagExpChatAlwaysRunTCS on, as after the legacy chat sunset).
- Left: clicking the Dialog's "?" (a button in a PlayerGui-held
  BillboardGui) doesn't fire MouseButton1Click in Play (gui_input
  hit-testing), so a conversation can't be started by mouse yet.
