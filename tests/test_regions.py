import regions
import test_app
class RegionTests(test_app.LedgerTests):
 def test_region_selection(self):
  self.assertEqual(len(regions.query()),31)
  self.assertTrue(any(r['name']=='杭州市' for r in regions.query('33')))
  city=next(r for r in regions.query('33') if r['name']=='杭州市')
  self.assertEqual(self.post('/api/addresses',{'region_code':city['code']}),200)
  import app
  with app.connect() as c:
   a=c.execute('SELECT * FROM addresses').fetchone();self.assertEqual(a['name'],city['path']);self.assertEqual(a['region_code'],city['code'])
  self.assertEqual(self.post('/api/addresses',{'region_code':'33'}),400)
  self.assertEqual(self.post('/api/addresses',{'region_code':'invalid'}),400)
  self.assertTrue(regions.query(q='浙江 杭州'))
