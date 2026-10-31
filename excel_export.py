"""Dependency-free OOXML export for the local ledger application."""
import io,zipfile
from xml.sax.saxutils import escape

def workbook(rows):
 data=[['日期','车牌','收支','项目','收入（元）','支出（元）','起点','终点','往来方','备注']]
 income=expense=0
 for r in rows:
  is_income=r['kind']=='收入';amount=r['cents']/100
  income+=r['cents'] if is_income else 0;expense+=r['cents'] if not is_income else 0
  data.append([r['date'],r['plate'] or r['name'],r['kind'],r['category'],amount if is_income else '',amount if not is_income else '',r['start_address'] or '未记录',r['end_address'] or '未记录',r['party'] or '',r['note'] or ''])
 data.extend([[],['合计收入','','','',income/100],['合计支出','','','','',expense/100],['收支差额','','','',(income-expense)/100]])
 xml=[]
 for i,row in enumerate(data,1):
  cells=[]
  for j,v in enumerate(row):
   ref=chr(65+j)+str(i);numeric=isinstance(v,(int,float));style=1 if i==1 else (2 if j==4 and numeric and (i!=len(data) or v>=0) else 3 if j==5 and numeric or i==len(data) and j==4 and numeric and v<0 else 0)
   cells.append(f'<c r="{ref}" s="{style}"'+(f'><v>{v:.2f}</v></c>' if numeric else f' t="inlineStr"><is><t xml:space="preserve">{escape(str(v))}</t></is></c>'))
  xml.append(f'<row r="{i}">'+''.join(cells)+'</row>')
 sheet='<worksheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><sheetViews><sheetView workbookViewId="0"><pane ySplit="1" topLeftCell="A2" activePane="bottomLeft" state="frozen"/></sheetView></sheetViews><cols>'+''.join(f'<col min="{j}" max="{j}" width="{w}" customWidth="1"/>' for j,w in enumerate([14,20,10,14,18,18,40,40,20,55],1))+'</cols><sheetData>'+''.join(xml)+f'</sheetData><autoFilter ref="A1:J{len(rows)+1}"/></worksheet>'
 styles='''<styleSheet xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"><fonts count="4"><font><sz val="11"/><name val="Calibri"/></font><font><b/><color rgb="FFFFFFFF"/></font><font><color rgb="FF148365"/></font><font><color rgb="FFBF6630"/></font></fonts><fills count="5"><fill><patternFill patternType="none"/></fill><fill><patternFill patternType="gray125"/></fill><fill><patternFill patternType="solid"><fgColor rgb="FF1E3A6B"/></patternFill></fill><fill><patternFill patternType="solid"><fgColor rgb="FFEAF7F1"/></patternFill></fill><fill><patternFill patternType="solid"><fgColor rgb="FFFFF2E9"/></patternFill></fill></fills><borders count="1"><border/></borders><cellStyleXfs count="1"><xf/></cellStyleXfs><cellXfs count="4"><xf fontId="0" fillId="0" borderId="0"/><xf fontId="1" fillId="2" borderId="0" applyFill="1" applyFont="1"/><xf numFmtId="4" fontId="2" fillId="3" borderId="0" applyNumberFormat="1" applyFont="1" applyFill="1"/><xf numFmtId="4" fontId="3" fillId="4" borderId="0" applyNumberFormat="1" applyFont="1" applyFill="1"/></cellXfs><cellStyles count="1"><cellStyle name="Normal" xfId="0" builtinId="0"/></cellStyles></styleSheet>'''
 files={'[Content_Types].xml':'<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types"><Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/><Default Extension="xml" ContentType="application/xml"/><Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/><Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/><Override PartName="/xl/styles.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.styles+xml"/></Types>', '_rels/.rels':'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/></Relationships>', 'xl/workbook.xml':'<workbook xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main" xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"><sheets><sheet name="收支明细" sheetId="1" r:id="rId1"/></sheets></workbook>', 'xl/_rels/workbook.xml.rels':'<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships"><Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/><Relationship Id="rId2" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/styles" Target="styles.xml"/></Relationships>', 'xl/worksheets/sheet1.xml':sheet,'xl/styles.xml':styles}
 out=io.BytesIO()
 with zipfile.ZipFile(out,'w',zipfile.ZIP_DEFLATED) as z:
  for name,content in files.items():z.writestr(name,content)
 return out.getvalue()
