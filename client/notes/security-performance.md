## Security

- Ensure that public github repo can't let random people from using the production server and run up the cost.
- Add security features to production server like rate limiting and max concurrent user count.

## Performance

# Fix fps drop

1. Measure first (no code needed). In settings, turn on DEBUG, then turn on the ENTITIES and DRAW CALLS readouts, plus CPU and MEMORY. Note the numbers at a quiet moment around rounds 5, 10, 15 and 20, then at round 21 in a fresh game.
   - If ENTITIES or MEMORY keep climbing across rounds, there's a real leak I haven't found, and the readout narrows down where to look.

# Performance

Debug mode on before game starts.
Benchmark (compare with the debug-mode PERFORMANCE report on the results screen; reports in `~/.config/bevy-trickshot/perf/`): Der Tempel night, Zombies, FULL POWER on, start round 21 → call the exfil at once, run to it, kill only with the Ray Gun (no nuke), win the exfil. Full screen, only client and server terminals running and nvim with these notes on workspace 4. No other applications open.

Baseline 2026-10-22 (v0.4.8, after zombie frustum culling; 3840x2160, shadows EXTRA, 240 cap): avg 146 FPS, 1% low 101, 0.1% low 83; 30+ enemies 1% low 98; ~6,700 entities idle → 14,700 at ~52 enemies (~150 per enemy); draw calls ~950 → ~2,600 (~33 per enemy). Shadow Map setting then only changed the moon: NORMAL vs EXTRA was avg 166 vs 155, 1% low 113 vs 101 (after the zombie mesh merge + footstep cap). With only the nearest power lights casting shadows (EXTRA): avg 162, 1% low 111, 0.1% low 89; 40+ enemies 152 FPS vs 161 with under 5. It doesn't cover the "worse after many rounds" problem — that needs a long run from round 1 (watch ENTITIES AT ROUND START).

### Before / after comparison (e.g. a Bevy upgrade)

Do this exact routine before the change, then again after, so the two sets of reports compare fairly.

1. Use release builds against a local server, both times: `cargo run --release -p server` and `cargo run --release -p bevy-trickshot`. Under `cargo` the client connects to `127.0.0.1` (`net::server_addr`), and a version bump would leave the prod server on the old protocol. Dev builds run our own crates at opt-level 1, which skews the numbers.
2. Use the same settings: 3840x2160, shadows EXTRA, 240 cap, vsync off, DEBUG on before the game starts. The report only flags RESOLUTION / SHADOWS / FPS CAP changes, so write down anything else that's been changed (FOV, render scale…) and keep it the same.
3. Same machine state: other heavy apps closed (browser, Discord), same monitor.
4. Play a warm-up game first: Zombies on Der Tempel from round 1, play about a minute, leave. That gets shader / pipeline compiling out of the way; the recorder's 2 s warm-up isn't enough after a fresh build. Starting at round 1 keeps it out of the comparison, because the panel compares against the last run with the same map, mode and starting round.
5. The benchmark above (round 21, exfil, Ray Gun only, no nuke, win the exfil so the results screen saves it). Take the same route to the exfil every time.
6. Run it 3 times and compare the middle run of each set, since hordes vary.
7. Before the change, copy the reports from `~/.config/bevy-trickshot/perf/` into e.g. `perf/pre-bevy-0.20/` and note the commit hash.
8. After the change, the first run's PERFORMANCE panel shows how each number moved since the last pre-change run. Compare: avg / 1% low / 0.1% low, the 30+ enemies section (fairest when hordes differ), peak draw calls (where the Bevy 0.19 batching should show), peak entities (should stay about the same; if not, the runs weren't like for like) and peak memory (won't move until textures are compressed with 0.20's `CompressedImageSaver`).

### Bevy upgrade benchmark runs

The benchmark above, one Bevy version at a time. Report files are in `~/.config/bevy-trickshot/perf/` (`<id>_dertempel_zombies.json` / `.csv`). All runs were dev builds (`cargo run`), at 3840x2160, shadows EXTRA, 240 cap. Every report says v0.4.10, so the version field doesn't tell these apart; the Bevy version is from the build each one ran on.

| Bevy | Report id | When | Avg | 1% low | 0.1% low | p99 ms | 30+ enemies avg / 1% | Peak draws | Peak entities | Peak mem |
|---|---|---|---|---|---|---|---|---|---|---|
| 0.16 | 1794964847 | 2026-11-17 19:20 | 119.5 | 62.8 | 54.7 | 13.7 | 107.2 / 58.6 | 1839 | 11301 | 13.8 GB |
| 0.16 | 1794964947 | 2026-11-17 19:22 | 111.0 | 56.6 | 47.2 | 15.1 | 100.9 / 61.1 | 2032 | 11439 | 13.9 GB |
| 0.16 | 1794965036 | 2026-11-17 19:23 | 96.4 | 53.0 | 45.1 | 17.7 | 82.1 / 51.3 | 2238 | 11674 | 13.9 GB |
| 0.17 | 1795139614 | 2026-11-19 19:53 | 112.6 | 53.9 | 50.4 | 17.8 | 100.2 / 58.6 | 2149 | 11427 | 11.0 GB |
| 0.18 | 1795142710 | 2026-11-19 20:45 | 115.4 | 52.9 | 47.6 | 18.2 | 102.5 / 54.3 | 1861 | *(broken)* | 13.6 GB |

- 0.16 = `main` before the upgrade (commit c148242). 0.17 = PR #1 (aecdba9). 0.18 = branch `bevy-0.18-upgrade`, client built 20:41 with bevy 0.18.1 / lightyear 0.26.4.
- The 0.18 run's entity counts are wrong: Bevy 0.18 changed `Entities::len()` to count every entity slot ever allocated, so the readout stuck at 11,976. Both the readout and the report now use `count_spawned()`, so later runs are fine.
- Not benchmarks (round 1 warm-ups, or left before the results screen): 1795139288, 1795139366, 1795139539 (0.17), and 1795142607, 1795142632 (0.18).
- The one-shot Bevy 0.20 attempt (shelved branch `bevy-0.20-upgrade`, reports say v0.4.11): 1794974993 (benchmark, avg 49.6) and 1794974878 (aborted at round 1). Runs from the lightyear 0.27 step onward also say v0.4.11 (the 4K skies started that version), so tell them apart by date.
- Starting with the lightyear 0.27 step, the skies are 4K instead of 8K (384 MiB less VRAM, ~0.65 GB lower RAM spike while one loads). Memory numbers before and after that aren't like for like.

## Tools and tests

- Add CPU vs GPU frame time to the perf report, to know which kind of fix will actually move the numbers
- Long run from round 1 with debug mode on, to check whether anything piles up over rounds (the by-round chart and ENTITIES AT ROUND START)
- When using phd slider a lot on a higher round with a lot of zombies, fps dropped to around 70-90. This seems to just be the case after going through this many rounds instead of starting on a high round from the start.
- Could run btop while game is running
