import sqlite3
import test_app
class RouteTests(test_app.LedgerTests):
 def address(self,name):
  self.assertEqual(self.post('/api/addresses',{'name':name,'detail':'详细地址'}),200)
  with self.connection() as c:return c.execute('SELECT id FROM addresses WHERE name=?',(name,)).fetchone()[0]
 def connection(self):
  import app
  return app.connect()
 def test_routes_and_history(self):
  self.post('/api/cars',{'name':'车','plate':'测试001'})
  a=self.address('上海');b=self.address('杭州')
  e={'car_id':1,'date':'2026-10-31','kind':'收入','category':'运费','amount':'1000'}
  self.assertEqual(self.post('/api/entries',e),400)
  e['start_address_id']=a;self.assertEqual(self.post('/api/entries',e),400)
  e['end_address_id']=b;self.assertEqual(self.post('/api/entries',e),200)
  self.assertEqual(self.post('/api/addresses',{'id':a,'name':'新上海','detail':'改过'}),200)
  self.assertEqual(self.post('/api/addresses/delete',{'id':b}),200)
  self.assertEqual(self.post('/api/entries',e),400)
  with self.connection() as c:
   row=c.execute('SELECT * FROM entries').fetchone();self.assertEqual(row['start_address'],'上海 / 详细地址');self.assertEqual(row['end_address'],'杭州 / 详细地址')
   cat=c.execute("SELECT id FROM categories WHERE name='运费'").fetchone()[0]
  self.post('/api/categories',{'id':cat,'kind':'收入','name':'运输收入'})
  e.update(category='运输收入',start_address_id=None,end_address_id=None);self.assertEqual(self.post('/api/entries',e),400)
  e.update(kind='支出',category='工资');self.assertEqual(self.post('/api/entries',e),200)
  e.update(start_address_id=a,end_address_id=a);self.assertEqual(self.post('/api/entries',e),200)
  self.assertEqual(self.post('/api/addresses',{'name':'新上海'}),400)
  import app,io,zipfile,xml.etree.ElementTree as ET
  h=object.__new__(app.App);h.path='/api/export';out=[];h.send=lambda d,**kw:out.append(d);h.do_GET()
  with zipfile.ZipFile(io.BytesIO(out[0])) as z:
   sheet=ET.fromstring(z.read('xl/worksheets/sheet1.xml'))
   text=' '.join(sheet.itertext());self.assertIn('上海 / 详细地址',text);self.assertIn('杭州 / 详细地址',text)
   self.assertIn('收入（元）',text);self.assertIn('支出（元）',text)

 def test_legacy_migration(self):
  import app
  with sqlite3.connect(app.DB) as c:
   c.executescript("CREATE TABLE cars(id INTEGER PRIMARY KEY,name TEXT NOT NULL,plate TEXT,vin TEXT,note TEXT);CREATE TABLE entries(id INTEGER PRIMARY KEY,car_id INTEGER,date TEXT,kind TEXT,category TEXT,cents INTEGER,party TEXT,note TEXT);INSERT INTO cars VALUES(1,'旧车','','','');INSERT INTO entries VALUES(1,1,'2026-10-31','收入','运费',12345,'','');")
  for _ in range(2):
   with self.connection() as c:
    row=c.execute('SELECT * FROM entries').fetchone();self.assertEqual((row['car_id'],row['cents'],row['start_address']),(1,12345,None))
    self.assertEqual(c.execute("SELECT requires_route FROM categories WHERE name='运费'").fetchone()[0],1)
