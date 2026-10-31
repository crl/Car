import test_app,app
class PlateTests(test_app.LedgerTests):
 def test_plate_required_and_model_optional(self):
  self.assertEqual(self.post('/api/cars',{'name':'仅车型'}),400)
  self.assertEqual(self.post('/api/cars',{'plate':'  '}),400)
  self.assertEqual(self.post('/api/cars',{'plate':'沪A12345'}),200)
  self.assertEqual(self.post('/api/cars',{'id':1,'plate':'沪B12345','note':'更新'}),200)
  with app.connect() as c:
   row=c.execute('SELECT * FROM cars WHERE id=1').fetchone()
   self.assertEqual(row['plate'],'沪B12345');self.assertEqual(row['name'],'')
 def test_legacy_model_preserved(self):
  self.post('/api/cars',{'plate':'沪A12345','name':'旧车型'})
  self.post('/api/cars',{'id':1,'plate':'沪B12345'})
  with app.connect() as c:self.assertEqual(c.execute('SELECT name FROM cars WHERE id=1').fetchone()[0],'旧车型')
