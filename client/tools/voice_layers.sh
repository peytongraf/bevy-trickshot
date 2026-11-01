#!/usr/bin/env bash
# Bakes the zombie-voice pitch layers into spoken lines.
#
# Every clean recording in client/voice_src/<voice>/ (mp3, wav, flac or ogg)
# gets two quieter copies of itself mixed underneath, one pitched up and one
# pitched down by SEMITONES. They're the same length as the original, so
# they stay locked to it. The result is written as an mp3 of the same name
# to the voice's folder in the game's assets (VOICES below). The echo and
# reverb aren't baked in: the game adds those as the line plays (debug panel
# → Audio → Voice effect), so the line keeps its exact length.
#
# Usage, from anywhere in the repo:
#   client/tools/voice_layers.sh          # only lines that are new or changed
#   client/tools/voice_layers.sh --all    # every line again (after changing the settings)
#
# Needs ffmpeg built with rubberband (Arch: the `ffmpeg` package has it).

set -euo pipefail

# --- settings -----------------------------------------------------------
# How far the two layers are pitched up and down.
SEMITONES=3
# How loud each layer is, next to the clean voice (dB).
UP_DB=-7
DOWN_DB=-5
# What happens to each layer's formants (the voice's character):
# "shifted" moves them with the pitch, for the classic small-voice-up /
# giant-voice-down sound; "preserved" keeps them, for a more natural voice
# at the new pitch.
FORMANTS=shifted
# mp3 quality (LAME VBR: 0 = best … 9 = smallest).
MP3_QUALITY=2

# Where each voice's clean recordings are (under client/voice_src/), and
# where its finished lines go (under client/assets/audio/).
declare -A VOICES=(
    [hierophant]="zombies/easter_egg/hierophant/voice"
)
# ------------------------------------------------------------------------

client="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
script="$client/tools/voice_layers.sh"
all=false
[[ "${1:-}" == "--all" ]] && all=true

if ! grep -q ' rubberband ' <<<"$(ffmpeg -hide_banner -filters 2>/dev/null)"; then
    echo "ffmpeg here doesn't have the rubberband filter" >&2
    exit 1
fi

up=$(awk -v s="$SEMITONES" 'BEGIN { printf "%.6f", 2 ^ (s / 12) }')
down=$(awk -v s="$SEMITONES" 'BEGIN { printf "%.6f", 2 ^ (-s / 12) }')
filter="[0:a]aformat=sample_fmts=fltp,asplit=3[dry][a][b];\
[a]rubberband=pitch=$up:formant=$FORMANTS:pitchq=quality,volume=${UP_DB}dB[up];\
[b]rubberband=pitch=$down:formant=$FORMANTS:pitchq=quality,volume=${DOWN_DB}dB[down];\
[dry][up][down]amix=inputs=3:duration=first:normalize=0,alimiter=limit=0.97:level=false[out]"

made=0
skipped=0
for voice in "${!VOICES[@]}"; do
    src="$client/voice_src/$voice"
    out="$client/assets/audio/${VOICES[$voice]}"
    mkdir -p "$out"
    shopt -s nullglob
    for clean in "$src"/*.{mp3,wav,flac,ogg}; do
        name="$(basename "${clean%.*}")"
        dest="$out/$name.mp3"
        # Up to date: newer than both its recording and this script.
        if ! $all && [[ -f "$dest" && "$dest" -nt "$clean" && "$dest" -nt "$script" ]]; then
            skipped=$((skipped + 1))
            continue
        fi
        ffmpeg -hide_banner -loglevel error -y -i "$clean" -filter_complex "$filter" -map "[out]" \
            -c:a libmp3lame -q:a "$MP3_QUALITY" "$dest"
        secs=$(ffprobe -v error -show_entries format=duration -of csv=p=0 "$dest")
        printf '  %-28s %6.2f s\n' "$voice/$name.mp3" "$secs"
        made=$((made + 1))
    done
done
echo "made $made, $skipped already up to date"
