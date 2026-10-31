import json
from pathlib import Path
TREE=json.loads((Path(__file__).parent/'data/regions.json').read_text())
INDEX={}
def walk(nodes,parent='',path=(),level=1):
 for node in nodes:
  names=path+(node['name'],)
  INDEX[node['code']]={'code':node['code'],'name':node['name'],'parent':parent,'level':level,'path':' / '.join(names),'has_children':bool(node.get('children'))}
  walk(node.get('children',[]),node['code'],names,level+1)
walk(TREE)
def query(parent='',q=''):
 if q:return [r for r in INDEX.values() if r['level']>=2 and all(word in r['path'] for word in q.split())][:100]
 return [r for r in INDEX.values() if r['parent']==parent]
