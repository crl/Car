import csv, io, json, os, sqlite3
from urllib.parse import urlparse, parse_qs
import regions
from excel_export import workbook
from http.server import BaseHTTPRequestHandler, HTTPServer
from pathlib import Path
ROOT=Path(__file__).parent
DB=Path(os.environ.get('CAR_DB',str(ROOT/'cars.db')))
def connect():
 c=sqlite3.connect(DB); c.row_factory=sqlite3.Row
 c.execute('PRAGMA foreign_keys=ON')
 c.executescript('CREATE TABLE IF NOT EXISTS cars(id INTEGER PRIMARY KEY,name TEXT NOT NULL,plate TEXT,vin TEXT,note TEXT); CREATE TABLE IF NOT EXISTS entries(id INTEGER PRIMARY KEY,car_id INTEGER NOT NULL REFERENCES cars(id),date TEXT NOT NULL,kind TEXT NOT NULL,category TEXT NOT NULL,cents INTEGER NOT NULL,party TEXT,note TEXT);')
 c.execute('CREATE TABLE IF NOT EXISTS categories(id INTEGER PRIMARY KEY,kind TEXT NOT NULL,name TEXT NOT NULL,active INTEGER NOT NULL DEFAULT 1,UNIQUE(kind,name))')
 c.execute('CREATE TABLE IF NOT EXISTS settings(key TEXT PRIMARY KEY)')
 if not c.execute("SELECT 1 FROM settings WHERE key='categories_seeded'").fetchone():
  c.executemany('INSERT OR IGNORE INTO categories(kind,name) VALUES(?,?)', [('收入','运费'),('支出','油费'),('支出','通行费'),('支出','倒板费'),('支出','工资')])
  c.execute("INSERT INTO settings VALUES('categories_seeded')")
  c.commit()
 c.execute('CREATE TABLE IF NOT EXISTS addresses(id INTEGER PRIMARY KEY,name TEXT NOT NULL,detail TEXT NOT NULL DEFAULT "",active INTEGER NOT NULL DEFAULT 1)')
 c.execute('CREATE UNIQUE INDEX IF NOT EXISTS addresses_active_name ON addresses(name) WHERE active=1')
 if 'requires_route' not in [r['name'] for r in c.execute('PRAGMA table_info(categories)')]:
  c.execute('ALTER TABLE categories ADD COLUMN requires_route INTEGER NOT NULL DEFAULT 0')
  c.execute("UPDATE categories SET requires_route=1 WHERE kind='收入' AND name='运费'")
 columns={r['name'] for r in c.execute('PRAGMA table_info(entries)')}
 for name,typ in [('start_address_id','INTEGER REFERENCES addresses(id)'),('end_address_id','INTEGER REFERENCES addresses(id)'),('start_address','TEXT'),('end_address','TEXT')]:
  if name not in columns: c.execute('ALTER TABLE entries ADD COLUMN '+name+' '+typ)
 if 'region_code' not in [r['name'] for r in c.execute('PRAGMA table_info(addresses)')]: c.execute('ALTER TABLE addresses ADD COLUMN region_code TEXT')
 c.commit()
 return c
class App(BaseHTTPRequestHandler):
 def send(self,data,status=200,typ='application/json; charset=utf-8'):
  if not isinstance(data,bytes): data=json.dumps(data,ensure_ascii=False).encode()
  self.send_response(status); self.send_header('Content-Type',typ); self.send_header('Content-Length',str(len(data))); self.end_headers(); self.wfile.write(data)
 def do_GET(self):
  if self.path in ['/', '/cars', '/entries', '/categories', '/addresses'] or __import__('re').fullmatch(r'/cars/[0-9]+',self.path): return self.send((ROOT/'index.html').read_bytes(),typ='text/html; charset=utf-8')
  if self.path in ['/style.css','/app.js']:
   return self.send((ROOT/self.path[1:]).read_bytes(),typ='text/css; charset=utf-8' if self.path.endswith('.css') else 'text/javascript; charset=utf-8')
  if urlparse(self.path).path=='/api/regions':
   params=parse_qs(urlparse(self.path).query)
   return self.send({'items':regions.query(params.get('parent',[''])[0],params.get('q',[''])[0].strip()),'version':'2023-06-30','coverage':'大陆31省级地区（不含港澳台）'})
  with connect() as c:
   if self.path=='/api/data': return self.send({t:[dict(r) for r in c.execute('SELECT * FROM '+t+' ORDER BY id DESC')] for t in ['cars','entries','categories','addresses']})
   if self.path=='/api/export':
    return self.send(workbook(list(c.execute('SELECT e.*,c.name,c.plate FROM entries e JOIN cars c ON c.id=e.car_id ORDER BY date,id'))),typ='application/vnd.openxmlformats-officedocument.spreadsheetml.sheet')
  self.send({'error':'不存在'},404)
 def do_POST(self):
  try:
   n=int(self.headers.get('Content-Length','0'))
   if n>100000: raise ValueError('内容过长')
   d=json.loads(self.rfile.read(n))
   with connect() as c:
    if self.path=='/api/export/xlsx':
     ids=d['ids']
     if not isinstance(ids,list) or len(ids)>10000: raise ValueError('导出记录过多')
     indexed={r['id']:r for r in c.execute('SELECT e.*,c.name,c.plate FROM entries e JOIN cars c ON c.id=e.car_id')}
     return self.send(workbook([indexed[int(i)] for i in ids]),typ='application/vnd.openxmlformats-officedocument.spreadsheetml.sheet')
    if self.path=='/api/cars':
     if not str(d.get('plate','')).strip(): raise ValueError('请填写车牌')
     if d.get('id') and 'name' not in d:
      existing=c.execute('SELECT name FROM cars WHERE id=?',(int(d['id']),)).fetchone()
      if not existing: raise ValueError('车辆不存在')
      d['name']=existing['name']
     values=[str(d.get(k,'')).strip() for k in ['name','plate','vin','note']]
     if d.get('id'): c.execute('UPDATE cars SET name=?,plate=?,vin=?,note=? WHERE id=?',values+[int(d['id'])])
     else: c.execute('INSERT INTO cars(name,plate,vin,note) VALUES(?,?,?,?)',values)
    elif self.path=='/api/categories':
     kind=d.get('kind'); name=str(d.get('name','')).strip()
     if kind not in ['收入','支出'] or not name or len(name)>60: raise ValueError('类型无效')
     if d.get('id'):
      if not c.execute('SELECT 1 FROM categories WHERE id=? AND active=1',(int(d['id']),)).fetchone(): raise ValueError('类型不存在')
      old=c.execute('SELECT requires_route FROM categories WHERE id=?',(int(d['id']),)).fetchone()[0]
      c.execute('UPDATE categories SET kind=?,name=?,requires_route=? WHERE id=?',(kind,name,int(bool(d.get('requires_route',old))),int(d['id'])))
     else:
      c.execute('INSERT INTO categories(kind,name,requires_route) VALUES(?,?,?) ON CONFLICT(kind,name) DO UPDATE SET active=1,requires_route=excluded.requires_route',(kind,name,int(bool(d.get('requires_route',False)))))
    elif self.path=='/api/categories/delete':
     c.execute('UPDATE categories SET active=0 WHERE id=?',(int(d['id']),))
    elif self.path=='/api/addresses':
     name=str(d.get('name','')).strip();detail=str(d.get('detail','')).strip()
     code=d.get('region_code')
     if code is not None:
      region=regions.INDEX.get(str(code))
      if not region or region['level']<2: raise ValueError('请选择城市、区县或乡镇/街道')
      name=region['path']; detail=''
     if not name: raise ValueError('请填写地址名称')
     if c.execute('SELECT 1 FROM addresses WHERE active=1 AND name=? AND id!=?',(name,int(d.get('id') or 0))).fetchone(): raise ValueError('地址名称已存在')
     if d.get('id'):
      if not c.execute('SELECT 1 FROM addresses WHERE id=? AND active=1',(int(d['id']),)).fetchone(): raise ValueError('地址已删除，请刷新')
      c.execute('UPDATE addresses SET name=?,detail=?,region_code=? WHERE id=?',(name,detail,code,int(d['id'])))
     else: c.execute('INSERT INTO addresses(name,detail,region_code) VALUES(?,?,?)',(name,detail,code))
    elif self.path=='/api/addresses/delete':
     if not c.execute('UPDATE addresses SET active=0 WHERE id=? AND active=1',(int(d['id']),)).rowcount: raise ValueError('地址已删除，请刷新')
    elif self.path=='/api/entries':
     from decimal import Decimal
     from datetime import date
     amount=Decimal(str(d['amount']))
     if not amount.is_finite() or amount<=0 or amount>Decimal('999999999') or amount*100!=(amount*100).to_integral_value(): raise ValueError('金额须大于零，最多两位小数')
     date.fromisoformat(d['date'])
     if d['kind'] not in ['收入','支出'] or not d['category'].strip(): raise ValueError('请填写有效收支项目')
     if not c.execute('SELECT 1 FROM categories WHERE kind=? AND name=? AND active=1',(d['kind'],d['category'].strip())).fetchone(): raise ValueError('请选择有效类型')
     category=c.execute('SELECT requires_route FROM categories WHERE kind=? AND name=? AND active=1',(d['kind'],d['category'].strip())).fetchone()
     start=d.get('start_address_id') or None;end=d.get('end_address_id') or None
     if bool(start)!=bool(end) or (category['requires_route'] and not start): raise ValueError('请同时选择起点和终点，此类型可能要求必填路线')
     snapshots=[]
     for address_id in [start,end]:
      if address_id is None: snapshots.append(None);continue
      address=c.execute('SELECT * FROM addresses WHERE id=? AND active=1',(int(address_id),)).fetchone()
      if not address: raise ValueError('所选地址已删除，请刷新后重新选择')
      snapshots.append(address['name']+(' / '+address['detail'] if address['detail'] else ''))
     c.execute('INSERT INTO entries(car_id,date,kind,category,cents,party,note,start_address_id,end_address_id,start_address,end_address) VALUES(?,?,?,?,?,?,?,?,?,?,?)',(int(d['car_id']),d['date'],d['kind'],d['category'].strip(),int(amount*100),d.get('party',''),d.get('note',''),start,end,*snapshots))
    else: return self.send({'error':'不存在'},404)
   self.send({'ok':True})
  except ValueError as e: self.send({'error':str(e)},400)
  except (KeyError,TypeError,sqlite3.Error,ArithmeticError): self.send({'error':'保存失败，请检查车辆、日期、金额和必填字段'},400)
if __name__=='__main__':
 connect().close()
 print('汽车内账：http://127.0.0.1:8765',flush=True)
 HTTPServer(('127.0.0.1',8765),App).serve_forever()
