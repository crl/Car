import unittest,tempfile,sqlite3
from pathlib import Path
import app,storage
class StorageTests(unittest.TestCase):
 def test_round_trip_and_safety_backup(self):
  with tempfile.TemporaryDirectory() as folder:
   db=Path(folder)/'test.db'
   with app.connect(db) as c:c.execute("INSERT INTO cars(name,plate) VALUES('','TEST1')")
   content=storage.snapshot(db)
   with app.connect(db) as c:c.execute("UPDATE cars SET plate='TEST2'")
   safety=storage.restore(db,content,Path(folder)/'backups',lambda path:app.connect(path).close())
   with sqlite3.connect(db) as c:self.assertEqual(c.execute('SELECT plate FROM cars').fetchone()[0],'TEST1')
   with sqlite3.connect(safety) as c:self.assertEqual(c.execute('SELECT plate FROM cars').fetchone()[0],'TEST2')
 def test_invalid_backup_preserves_data(self):
  with tempfile.TemporaryDirectory() as folder:
   db=Path(folder)/'test.db';app.connect(db).close();before=storage.snapshot(db)
   for content in [b'not sqlite',b'SQLite format 3\x00'+b'x'*200]:
    with self.assertRaises((ValueError,sqlite3.Error)):storage.restore(db,content,Path(folder)/'backups')
    self.assertEqual(storage.snapshot(db),before)
   self.assertFalse((Path(folder)/'backups').exists())
