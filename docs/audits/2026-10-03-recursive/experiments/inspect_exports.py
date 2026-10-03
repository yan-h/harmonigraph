"""Verify completed media streams and extract two local-only inspection frames."""
from pathlib import Path
import json,subprocess
here=Path(__file__).resolve().parent; out=here/'output'; logs=here.parent/'logs'; rows=[]
for name in ['settled-export-chord-1920x1080-1','settled-export-historical-1920x1080-0']:
    source=out/(name+'.mp4')
    cmd=['ffprobe','-v','error','-count_frames','-show_entries','format=duration:stream=codec_type,codec_name,width,height,nb_read_frames,sample_rate','-of','json',str(source)]
    data=json.loads(subprocess.check_output(cmd,text=True));rows.append({'name':name,'command':cmd,'streams':data})
    subprocess.run(['ffmpeg','-v','error','-ss','1.2','-i',str(source),'-frames:v','1','-y',str(out/(name+'.png'))],check=True)
(logs/'media-inspection.json').write_text(json.dumps(rows,indent=2)+'\n')
