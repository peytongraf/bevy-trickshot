## Security

- Ensure that public github repo can't let random people from using the production server and run up the cost.
- Add security features to production server like rate limiting and max concurrent user count.

## Performance

# Fix fps drop

1. Measure first (no code needed). In settings, turn on DEBUG, then turn on the ENTITIES and DRAW CALLS readouts, plus CPU and MEMORY. Note the numbers at a quiet moment around rounds 5, 10, 15 and 20, then at round 21 in a fresh game.
   - If ENTITIES or MEMORY keep climbing across rounds, there's a real leak I haven't found, and the readout narrows down where to look.

# Performance

Benchmark (compare with the debug-mode PERFORMANCE report on the results screen; reports in `~/.config/bevy-trickshot/perf/`): Der Tempel night, Zombies, FULL POWER on, start round 21 → call the exfil at once, run to it, kill only with the Ray Gun (no nuke), win the exfil. Baseline 2026-10-22 (v0.4.8, after zombie frustum culling; 3840x2160, shadows EXTRA, 240 cap): avg 146 FPS, 1% low 101, 0.1% low 83; 30+ enemies 1% low 98; ~6,700 entities idle → 14,700 at ~52 enemies (~150 per enemy); draw calls ~950 → ~2,600 (~33 per enemy). Shadow Map setting then only changed the moon: NORMAL vs EXTRA was avg 166 vs 155, 1% low 113 vs 101 (after the zombie mesh merge + footstep cap). With only the nearest power lights casting shadows (EXTRA): avg 162, 1% low 111, 0.1% low 89; 40+ enemies 152 FPS vs 161 with under 5. It doesn't cover the "worse after many rounds" problem — that needs a long run from round 1 (watch ENTITIES AT ROUND START).

## Tools and tests

- Add CPU vs GPU frame time to the perf report, to know which kind of fix will actually move the numbers
- Long run from round 1 with debug mode on, to check whether anything piles up over rounds (the by-round chart and ENTITIES AT ROUND START)
- When using phd slider a lot on a higher round with a lot of zombies, fps dropped to around 70-90. This seems to just be the case after going through this many rounds instead of starting on a high round from the start.
- Could run btop while game is running
