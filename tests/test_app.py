import io,json,sys,tempfile,unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
import app
class LedgerTests(unittest.TestCase):
 def setUp(self):
  self.tmp=tempfile.TemporaryDirectory();self.old=app.DB;app.DB=Path(self.tmp.name)/'test.db'
 def tearDown(self):app.DB=self.old;self.tmp.cleanup()
 def post(self,path,data):
  h=object.__new__(app.App);b=json.dumps(data).encode();h.path=path;h.headers={'Content-Length':str(len(b))};h.rfile=io.BytesIO(b);out=[];h.send=lambda data,status=200,**kw:out.append((status,data));h.do_POST();return out[0][0]
 def test_category_lifecycle_preserves_history(self):
  self.assertEqual(self.post('/api/cars',{'name':'测试车','plate':'测试001'}),200)
  self.assertEqual(self.post('/api/categories',{'kind':'支出','name':'维修费'}),200)
  with app.connect() as c: id=c.execute("SELECT id FROM categories WHERE name='维修费'").fetchone()[0]
  entry={'car_id':1,'date':'2026-10-31','kind':'支出','category':'维修费','amount':'12.34'}
  self.assertEqual(self.post('/api/entries',entry),200)
  self.assertEqual(self.post('/api/categories',{'id':id,'kind':'支出','name':'维修保养'}),200)
  self.assertEqual(self.post('/api/entries',entry),400)
  self.assertEqual(self.post('/api/categories/delete',{'id':id}),200)
  with app.connect() as c:
   row=c.execute('SELECT category,cents FROM entries').fetchone();self.assertEqual(tuple(row),('维修费',1234))
   self.assertEqual(c.execute('SELECT active FROM categories WHERE id=?',(id,)).fetchone()[0],0)
  self.assertEqual(self.post('/api/categories',{'kind':'支出','name':'维修保养'}),200)
  with app.connect() as c:self.assertEqual(c.execute('SELECT active FROM categories WHERE id=?',(id,)).fetchone()[0],1)
 def test_invalid_amount_and_wrong_kind(self):
  self.post('/api/cars',{'name':'车','plate':'测试001'})
  for amount in ['-1','0','1.001','NaN','Infinity']:
   self.assertEqual(self.post('/api/entries',{'car_id':1,'date':'2026-10-31','kind':'支出','category':'油费','amount':amount}),400)
  self.assertEqual(self.post('/api/entries',{'car_id':1,'date':'2026-10-31','kind':'收入','category':'油费','amount':'1'}),400)
if __name__=='__main__':unittest.main()
