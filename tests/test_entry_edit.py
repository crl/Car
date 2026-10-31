import test_app,app
class EntryEditTests(test_app.LedgerTests):
 def test_edit_in_place(self):
  self.post('/api/cars',{'plate':'测试车'})
  e={'car_id':1,'date':'2026-10-25','kind':'支出','category':'油费','amount':'100','note':'原备注'}
  self.assertEqual(self.post('/api/entries',e),200)
  e.update(id=1,amount='123.45',note='新备注')
  self.assertEqual(self.post('/api/entries',e),200)
  with app.connect() as c:
   self.assertEqual(c.execute('SELECT COUNT(*) FROM entries').fetchone()[0],1)
   r=c.execute('SELECT * FROM entries').fetchone();self.assertEqual(r['cents'],12345);self.assertEqual(r['note'],'新备注')
  e['amount']='-1';self.assertEqual(self.post('/api/entries',e),400)
  with app.connect() as c:self.assertEqual(c.execute('SELECT cents FROM entries').fetchone()[0],12345)
