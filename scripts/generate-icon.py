"""Code-drawn transport app icon, RGBA PNG without dependencies."""
import struct,zlib,math
from pathlib import Path
size=512
rows=[]
for py in range(size):
 y=(py-48)*512/416
 row=bytearray()
 for px in range(size):
  x=(px-48)*512/416
  dx=max(0,88-x,x-423);dy=max(0,88-y,y-423)
  inside=dx*dx+dy*dy<=88*88
  t=(x+y)/1024
  color=(int(155-42*t),int(120-38*t),int(226-30*t),255) if inside else (0,0,0,0)
  if inside:
   # Motion strokes, cargo body and cab form one clear transport silhouette.
   if (96<=x<157 and 223<=y<237) or (77<=x<142 and 259<=y<273):color=(228,211,255,255)
   if (169<=x<312 and 159<=y<306):color=(255,255,255,255)
   if 321<=x<405 and 213<=y<306 and (x<372 or y>213+(x-372)*1.15):color=(237,250,255,255)
   if 335<=x<375 and 229<=y<262:color=(130,93,193,255)
   if 154<=x<413 and 300<=y<322:color=(255,255,255,255)
   for cx in [208,363]:
    r=(x-cx)**2+(y-325)**2
    if r<28**2:color=(75,47,119,255)
    if r<13**2:color=(220,246,255,255)
   # Mint route arc and destination point beneath the vehicle.
   if 167<=x<=362 and abs(y-(385-0.0016*(x-258)**2))<5:color=(229,212,255,255)
   if (x-367)**2+(y-365)**2<10**2:color=(229,212,255,255)
  row.extend(color)
 rows.append(b'\0'+row)
def chunk(t,d):return struct.pack('!I',len(d))+t+d+struct.pack('!I',zlib.crc32(t+d)&0xffffffff)
Path('src-tauri/app-icon.png').write_bytes(b'\x89PNG\r\n\x1a\n'+chunk(b'IHDR',struct.pack('!IIBBBBB',size,size,8,6,0,0,0))+chunk(b'IDAT',zlib.compress(b''.join(rows)))+chunk(b'IEND',b''))
