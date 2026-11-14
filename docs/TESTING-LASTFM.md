# Testing Similarity Radio against the real Last.fm API

## Why this is a separate step

Aqua Glass was built in a sandbox whose network policy blocks `ws.audioscrobbler.com`
(connections are refused by the egress proxy). So far Similarity Radio has been verified with:

* unit tests of the Last.fm response parser, using the JSON shapes from Last.fm's
  documentation (including numbers sent as strings);
* unit tests of the gather/rank/resolve pipeline on a deterministic in-memory similarity graph;
* an end-to-end test (`tests/radio_e2e.rs`) with real audio playback, using the
  artist/album fallback source.

**It has not yet been run against real Last.fm responses.** The tools below do exactly that
on a PC with internet access. Each runs the same `radio::self_test`, which:

1. calls `track.getSimilar`, `artist.getSimilar` and `artist.getTopTracks` with your key;
2. checks the response shape:
   * the key is accepted;
   * similar tracks exist, with similarity scores between 0 and 1, ordered by similarity;
   * related artists exist;
   * top tracks include listener counts;
3. runs all four discovery modes and checks each one:
   * returns at least 10 recommendations;
   * never recommends the seed song itself;
   * in Deep Discovery, never recommends the seed's artist;
4. checks that the modes actually differ:
   * Familiar and Deep Discovery share less than half of their artists;
   * Adventurous draws from a varied set of artists;
5. lists the top 15 recommendations per mode, with scores and listener counts, so you can
   judge their quality yourself, and counts how many would play from your local library.

## 1. Get a Last.fm API key (free, ~2 minutes)

1. Sign in at <https://www.last.fm> (or create an account).
2. Open <https://www.last.fm/api/account/create>. Application name: "Aqua Glass (personal)";
   the callback URL can stay empty.
3. Copy the **API key**. You don't need the shared secret.

## 2a. Easiest: inside the app

Settings › Playback › Similarity Radio:

1. Paste the key and press **Save**. It is stored in Windows Credential Manager.
2. Under **Live self-test**, keep the suggested seed (the current song, or Radiohead — Karma
   Police) or type another, then press **Run self-test**.
3. A notification shows "N passed, M failed". The full report appears below the button
   (**Copy report** copies it) and is saved as
   `%APPDATA%\AquaGlass\radio-selftest-<time>.txt`.

## 2b. From PowerShell or Command Prompt, using the released exe (no Rust needed)

```powershell
cd C:\path\to\aqua-glass-0.3.0-windows-x64
$env:AQUA_LASTFM_KEY = "your-32-character-key"   # or skip if you saved it in the app
.\aqua-glass.exe --radio-selftest "Radiohead" "Karma Police"
echo $LASTEXITCODE    # 0 = every check passed
```

The report is printed in the terminal and saved to the data folder. Try a few seeds from
genres you listen to:

```powershell
.\aqua-glass.exe --radio-selftest "Daft Punk" "Get Lucky"
```

```powershell
.\aqua-glass.exe --radio-selftest "Nightcall" "Sleepwalking"
```

## 2c. With the Rust toolchain (developers)

```powershell
$env:AQUA_LASTFM_KEY = "your-key"
cargo test --release --test lastfm_live -- --ignored --nocapture
```

This runs three live tests:

* a well-known song;
* three different genres;
* a nonsense seed, which must be handled gracefully.

## 3. Then try the real thing

1. Add a local music folder or sign in to Spotify (Premium is needed for playback).
2. Search for a song and press Play.
3. Open the Queue panel (☰ in the player bar) › **Similarity Radio**. The source line should
   read **"Last.fm · Balanced"** (not "Artist & album based").
4. Switch Discovery Strength between Familiar and Deep Discovery and press **Refresh** to
   compare the recommendations.

## What to send back if something looks wrong

Send the saved `radio-selftest-*.txt` report. It contains no key or account data — only the
seed, the API result counts and the recommended artist/title pairs.

## Notes

* Only songs that resolve to something playable are queued: your local library first, then
  Spotify (when Premium playback is ready). With an empty library and no Spotify, the self-test
  still works (it only needs Last.fm). The live radio will report "No playable source".
* Last.fm rate limits are generous for personal use. Responses are cached (48 entries), so
  repeated refreshes of the same seed don't re-query.
* The sandbox can't use your key: Last.fm takes it as a URL parameter, so a proxy-injected
  header can't carry it. Testing on your PC is the reliable path.
