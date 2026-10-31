"""Validated SQLite snapshots for the local ledger."""
import sqlite3,tempfile
from pathlib import Path
from datetime import datetime

def snapshot(database):
 with tempfile.TemporaryDirectory() as folder:
  target=Path(folder)/'ledger.db'
  with sqlite3.connect(database) as source,sqlite3.connect(target) as dest:source.backup(dest)
  return target.read_bytes()

def restore(database,content,backup_dir,prepare=None):
 if not content.startswith(b'SQLite format 3\x00'):raise ValueError('请选择有效的 SQLite 账本备份文件')
 with tempfile.TemporaryDirectory() as folder:
  source_path=Path(folder)/'restore.db';source_path.write_bytes(content)
  with sqlite3.connect('file:'+str(source_path)+'?mode=ro',uri=True) as source:
   if source.execute('PRAGMA integrity_check').fetchone()[0]!='ok':raise ValueError('备份文件已损坏')
   required={'cars':{'id','name','plate','vin','note'},'entries':{'id','car_id','date','kind','category','cents','party','note'}}
   for table,columns in required.items():
    actual={r[1] for r in source.execute('PRAGMA table_info('+table+')')}
    if not columns.issubset(actual):raise ValueError('此文件不是兼容的运管家备份')
   if source.execute('PRAGMA foreign_key_check').fetchall():raise ValueError('备份中存在无效数据关联')
   if source.execute('SELECT 1 FROM entries e LEFT JOIN cars c ON c.id=e.car_id WHERE c.id IS NULL LIMIT 1').fetchone():raise ValueError('备份中存在无效车辆关联')
   if prepare:prepare(source_path)
   backup_dir=Path(backup_dir);backup_dir.mkdir(parents=True,exist_ok=True)
   safety=backup_dir/('before-restore-'+datetime.now().strftime('%Y%m%d-%H%M%S-%f')+'.db')
   safety.write_bytes(snapshot(database))
   with sqlite3.connect(database) as dest:source.backup(dest)
   return str(safety.resolve())
