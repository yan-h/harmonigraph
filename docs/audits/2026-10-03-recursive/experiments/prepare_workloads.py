"""Reproducible audit inputs; never writes to the user's take directory."""
from pathlib import Path
import hashlib, json, math, re, struct, wave
out = Path(__file__).resolve().parent / 'inputs'
out.mkdir(exist_ok=True)
rate = 48000
chords = [(48, 60, 64, 67, 72, 76), (50, 62, 65, 69, 74, 77), (43, 55, 59, 62, 67, 71), (48, 60, 64, 67, 72, 76)]
lines = ['Header((version:6,sample_rate:48000.0,source:"audit synthetic six-note progression",audio_file:Some("chord.wav"),audio_start:Some(0.0)))']
for i, chord in enumerate(chords):
    for note in chord:
        lines.append(f'Note((t:{i*2.0},source:0,channel:0,note:{note},kind:On(velocity:0.8)))')
    for note in chord:
        lines.append(f'Note((t:{i*2+1.8},source:0,channel:0,note:{note},kind:Off))')
(out/'chord.take').write_text('\n'.join(lines)+'\n')
with wave.open(str(out/'chord.wav'),'wb') as f:
    f.setnchannels(2); f.setsampwidth(2); f.setframerate(rate)
    for i, chord in enumerate(chords):
        data=bytearray()
        for n in range(rate*2):
            t=n/rate
            env=min(1,t/0.01)*min(1,max(0,(1.8-t)/0.15))
            v=sum(math.sin(2*math.pi*440*2**((note-69)/12)*t*h)/h for note in chord for h in (1,2,3,4))/48*env
            s=round(max(-1,min(1,v))*32767)
            data.extend(struct.pack('<hh',s,s))
        f.writeframes(data)
original=Path('/Users/yan/Music/Harmonigraph Takes/take-2026-09-25_22-16-27.take')
manifest={'synthetic': {'seconds':8, 'notes':24, 'simultaneous_notes':6, 'audio':'48kHz stereo PCM16 four-harmonic chords', 'appearance':'explicitly current defaults (no captured document)'}, 'historical':None}
if original.exists():
    text=original.read_text()
    headers=[line for line in text.splitlines() if line.startswith('Header(')]
    final_header=headers[-1]
    wav_name=re.search(r'audio_file:Some\("([^"]+)"\)',final_header).group(1)
    audio=original.parent/wav_name
    dest=out/'historical.wav'
    if not dest.exists(): dest.symlink_to(audio)
    def convert_header(header):
        start=re.search(r'audio_start:Some\(([^)]+)\)',header)
        sample_rate=re.search(r'sample_rate:([0-9.]+)',header).group(1)
        return f'Header((version:6,sample_rate:{sample_rate},source:"audit historical notes with current default appearance",audio_file:Some("historical.wav"),audio_start:Some({start.group(1) if start else "0.0"})))'
    migrated='\n'.join(convert_header(line) if line.startswith('Header(') else line.replace('participating:', 'shown:') for line in text.splitlines())+'\n'
    (out/'historical.take').write_text(migrated)
    manifest['historical']={'source':str(original),'sha256':hashlib.sha256(original.read_bytes()).hexdigest(),'audio':str(audio),'audio_bytes':audio.stat().st_size,'transformation':'scratch copy only: every header explicitly v6 with current-default appearance; original per-header sample rate and audio start retained; participating renamed shown; event, param, config lines otherwise identical', 'headers_converted':len(headers), 'field_renames':text.count('participating:')}
manifest['generated']={p.name:hashlib.sha256(p.read_bytes()).hexdigest() for p in out.iterdir() if p.is_file() and not p.is_symlink() and p.name != 'manifest.json'}
(out/'manifest.json').write_text(json.dumps(manifest,indent=2)+'\n')
print(json.dumps(manifest,indent=2))
