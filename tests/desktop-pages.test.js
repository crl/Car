import {pinyin} from 'pinyin-pro';
import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {JSDOM} from 'jsdom';
const html=readFileSync(new URL('../index.html',import.meta.url),'utf8');
const source=readFileSync(new URL('../app.js',import.meta.url),'utf8').replace(/^import .*;$/gm,'');
function fixture(){return {drivers:[{id:1,name:'张师傅',phone:'13800138000',note:'夜班',car_ids:[1,2]},{id:2,name:'李师傅',phone:'',note:'',car_ids:[]}],cars:[{id:1,plate:'测试001',name:'旧车型',vin:'VIN1',note:'备注'},{id:2,plate:'测试002',name:'',vin:'',note:''}],categories:[{id:1,kind:'收入',name:'运费',active:1,requires_route:1},{id:2,kind:'支出',name:'油费',active:1,requires_route:0}],addresses:[{id:1,name:'上海',detail:'',active:1},{id:2,name:'杭州',detail:'',active:1}],entries:Array.from({length:81},(_,i)=>({id:i+1,car_id:i===80?2:1,date:'2026-10-'+String(25+i%7).padStart(2,'0'),kind:i%2?'支出':'收入',category:i%2?'油费':'运费',cents:10000,start_address_id:i%3?1:null,end_address_id:i%3?2:null,start_address:i%3?'上海':null,end_address:i%3?'杭州':null,start_address_detail:i===1?'小区602室':'',end_address_detail:i===1?'园区8号':'',party:i%2?'油站':'客户',note:'备注'}))}}
async function page(route){const dom=new JSDOM(html,{url:'http://tauri.localhost/#'+route,runScripts:'outside-only'}),w=dom.window,calls=[],data=fixture();
 w.pinyin=pinyin;w.service={request:async()=>({ok:true,json:async()=>structuredClone(data)}),call:async(cmd,args)=>{calls.push({cmd,args});return {cancelled:true}}};w.confirmDialog=async()=>true;w.alertDialog=async()=>{};
 w.HTMLDialogElement.prototype.showModal=function(){this.open=true};w.HTMLDialogElement.prototype.close=function(){this.open=false;this.dispatchEvent(new w.Event('close'))};
 const matches=w.Element.prototype.matches;w.Element.prototype.matches=function(s){return s===':popover-open'?this.hasAttribute('data-open'):matches.call(this,s)};
 w.HTMLElement.prototype.showPopover=function(){this.setAttribute('data-open','')};w.HTMLElement.prototype.hidePopover=function(){this.removeAttribute('data-open')};
 await vm.runInContext('(async()=>{'+source+'})()',dom.getInternalVMContext());return {dom,w,calls,close:()=>w.close()};}
test('all desktop routes render and expose inline handlers',async()=>{for(const route of ['/cars','/entries','/cars/1','/categories','/addresses','/drivers','/cars/999']){const h=await page(route);try{assert.doesNotMatch(h.w.document.querySelector('main').textContent,/读取失败/);assert.equal(typeof h.w.entryForm,'function');assert.equal(typeof h.w.openSiteSettings,'function');if(route==='/cars/999')assert.match(h.w.document.querySelector('main').textContent,/车辆不存在/)}finally{h.close()}}});
test('ledger pagination, intersecting filters, clear and export use all filtered rows',async()=>{const h=await page('/entries'),w=h.w,d=w.document;try{
 assert.equal(d.querySelectorAll('#vehicleEntries tr').length,20);
 assert.match(d.querySelectorAll('.vehicle-entries th')[1].textContent,/车辆/);
 assert.equal(d.querySelector('#vehicleEntries tr a').getAttribute('href').startsWith('#/cars/'),true);
 w.changeVehiclePageSize(50);assert.equal(d.querySelectorAll('#vehicleEntries tr').length,50);
 w.goVehiclePage(2);assert.equal(d.querySelectorAll('#vehicleEntries tr').length,31);
 w.toggleAllKinds();assert.match(d.querySelector('#subtotal').textContent,/0 笔/);
 w.toggleKind({checked:true,value:'收入'});assert.match(d.querySelector('#subtotal').textContent,/41 笔/);
 w.allVehicleProjects();assert.match(d.querySelector('#subtotal').textContent,/0 笔/);
 w.toggleVehicleProject({checked:true,value:JSON.stringify(['收入','运费'])});assert.match(d.querySelector('#subtotal').textContent,/41 笔/);
 await w.exportVehicleRows();assert.equal(h.calls.at(-1).args.ids.length,41);
 w.clearFilters();assert.match(d.querySelector('#subtotal').textContent,/81 笔/);
 assert.equal(d.querySelectorAll('#vehicleEntries tr').length,50);
}finally{h.close()}});
test('entry editing retains values and route swap; cancel does not save',async()=>{const h=await page('/cars/1'),w=h.w,d=w.document;try{
 w.editLedgerEntry(1);const f=d.querySelector('#form');assert.equal(f.elements.car_id.value,'1');assert.equal(f.elements.amount.value,'100.00');assert.equal(f.elements.note.value,'备注');
 f.elements.start_address.value='上海';f.elements.end_address.value='杭州';w.swapRoute();assert.equal(f.elements.start_address.value,'杭州');assert.equal(f.elements.end_address.value,'上海');await w.requestClose();assert.equal(d.querySelector('#modal').open,false);assert.equal(h.calls.length,0);
}finally{h.close()}});
test('income and expense categories have independent search and fixed direction',async()=>{const h=await page('/categories'),w=h.w,d=w.document;try{assert.equal(d.querySelector('#category-group-0 tbody').textContent.includes('油费'),false);assert.equal(d.querySelector('#category-group-1 tbody').textContent.includes('运费'),false);w.filterCategories('无匹配',0);assert.equal(d.querySelector('#category-group-0 tbody tr').hidden,true);assert.equal(d.querySelector('#category-group-1 tbody tr').hidden,false);w.categoryForm(null,'收入');assert.equal(d.querySelector('#form').elements.kind.value,'收入');assert.equal(d.querySelector('#form select[name=kind]'),null);}finally{h.close()}});

test('route search includes every nationwide match and selects a region',async()=>{
 const h=await page('/cars/1'),w=h.w,d=w.document;
 try{
  w.entryForm(1);
  const regions=Array.from({length:125},(_,i)=>({code:String(i),name:'西湖街道',path:`地区${i} / 西湖街道`,level:4}));
  const addresses=fixture().addresses;
  w.service.request=async(path,options={})=>{
   if(path.startsWith('/api/regions?')){assert.equal(new URL(path,'http://local').searchParams.get('all'),'true');return {ok:true,json:async()=>({items:regions})};}
   if(path==='/api/addresses'){const region=regions.find(r=>r.code===JSON.parse(options.body).region_code);addresses.push({id:3,name:region.path,detail:'',active:1,region_code:region.code});}
   return {ok:true,json:async()=>({...fixture(),addresses})};
  };
  const input=d.querySelector('[name=start_address]');input.value='西湖';w.suggest('start');
  await new Promise(resolve=>setTimeout(resolve,230));
  assert.equal(d.querySelectorAll('#start-list [role=option]').length,125);
  d.querySelector('#start-list [role=option]').click();
  await new Promise(resolve=>setTimeout(resolve,20));
  assert.equal(input.value,'地区0 / 西湖街道');assert.equal(d.querySelector('#start-list').hidden,true);
 }finally{h.close()}
});

test('route search ignores stale responses and hidden lists',async()=>{
 const h=await page('/cars/1'),w=h.w,d=w.document;
 try{
  w.entryForm(1);const pending=[];
  w.service.request=path=>new Promise(resolve=>pending.push({path,resolve}));
  const input=d.querySelector('[name=start_address]');
  input.value='西湖';w.suggest('start');await new Promise(r=>setTimeout(r,210));
  input.value='杭州';w.suggest('start');await new Promise(r=>setTimeout(r,210));
  pending[1].resolve({ok:true,json:async()=>({items:[{code:'33',name:'杭州',path:'浙江 / 杭州'}]})});
  await new Promise(r=>setTimeout(r,10));
  pending[0].resolve({ok:true,json:async()=>({items:[{code:'1',name:'西湖',path:'旧结果 / 西湖'}]})});
  await new Promise(r=>setTimeout(r,10));
  assert.doesNotMatch(d.querySelector('#start-list').textContent,/旧结果/);
  input.value='西湖';w.suggest('start');await new Promise(r=>setTimeout(r,210));w.hideSuggestions('start');
  pending[2].resolve({ok:true,json:async()=>({items:[{code:'1',name:'西湖',path:'旧结果 / 西湖'}]})});
  await new Promise(r=>setTimeout(r,10));assert.equal(d.querySelector('#start-list').hidden,true);
 }finally{h.close()}
});

test('region search reports errors and selection can be retried with Enter',async()=>{
 const h=await page('/cars/1'),w=h.w,d=w.document;
 try{
  w.entryForm(1);const input=d.querySelector('[name=end_address]');input.value='西湖';
  w.service.request=async()=>{throw Error('查询失败')};w.suggest('end');await new Promise(r=>setTimeout(r,210));
  assert.match(d.querySelector('#end-list').textContent,/搜索失败/);
  let failed=true;const address={id:5,name:'浙江 / 杭州 / 西湖区',active:1,detail:'',region_code:'330106'};
  w.service.request=async(path)=>{
   if(path.startsWith('/api/regions?'))return {ok:true,json:async()=>({items:[{code:address.region_code,name:'西湖区',path:address.name}]})};
   if(path==='/api/addresses'&&failed){failed=false;throw Error('保存失败，请重试')}
   return {ok:true,json:async()=>({...fixture(),addresses:[...fixture().addresses,address]})};
  };
  w.suggest('end');await new Promise(r=>setTimeout(r,210));const option=d.querySelector('#end-list button');option.click();await new Promise(r=>setTimeout(r,10));
  assert.match(d.querySelector('#end-list').textContent,/保存失败/);assert.equal(d.querySelector('#save').disabled,false);
  option.setAttribute('aria-selected','true');w.addressKey({key:'Enter',target:input,preventDefault(){}},'end');
  await new Promise(r=>setTimeout(r,10));assert.equal(input.value,address.name);
  w.suggest('end');await new Promise(r=>setTimeout(r,210));assert.equal(d.querySelectorAll('#end-list [role=option]').length,1);
 }finally{h.close()}
});


test('route search matches common addresses by full pinyin and initials',async()=>{
 const h=await page('/cars/1'),w=h.w,d=w.document;
 try{
  w.entryForm(1);w.service.request=async()=>({ok:true,json:async()=>({items:[]})});
  const input=d.querySelector('[name=start_address]');
  for(const q of ['hangzhou','HANG ZHOU','hz','hang']){
   input.value=q;w.suggest('start');
   assert.match(d.querySelector('#start-list').textContent,/杭州/);
  }
 }finally{h.close()}
});


test('route detail inputs track dirty state and swap with regions',async()=>{
 const h=await page('/cars/1'),w=h.w,d=w.document;
 try{
  w.entryForm(1);const f=d.querySelector('#form').elements;
  assert.equal(f.start_address_detail.value,'');
  assert.ok(d.querySelector('.route-endpoint-heading button'));
  f.start_address.value='上海';f.end_address.value='杭州';
  f.start_address_detail.value='小区602室';f.end_address_detail.value='园区8号';
  assert.equal(w.frontendDirty(),true);w.swapRoute();
  assert.equal(f.start_address.value,'杭州');assert.equal(f.start_address_detail.value,'园区8号');
  assert.equal(f.end_address_detail.value,'小区602室');
  w.editLedgerEntry(1);assert.equal(d.querySelector('#form').elements.start_address_detail.value,'');
 }finally{h.close()}
});


test('details refill on edit and survive failed saves with full-address tooltips',async()=>{
 const h=await page('/cars/1'),w=h.w,d=w.document;
 try{
  w.changeVehiclePageSize(80);
  assert.ok([...d.querySelectorAll('#vehicleEntries span[title]')].some(e=>e.title==='上海 / 小区602室'&&e.textContent==='上海'));
  w.editLedgerEntry(2);const form=d.querySelector('#form'),f=form.elements;
  assert.equal(f.start_address_detail.value,'小区602室');assert.equal(f.end_address_detail.value,'园区8号');
  let saved;
  w.service.request=async(path,options)=>{saved=JSON.parse(options.body);throw Error('保存失败')};
  await form.onsubmit({preventDefault(){}});
  assert.equal(saved.start_address_detail,'小区602室');assert.equal(f.start_address_detail.value,'小区602室');assert.equal(d.querySelector('#modal').open,true);
  f.start_address.value='';await form.onsubmit({preventDefault(){}});
  assert.match(d.querySelector('#error-start_address').textContent,/请选择地区/);
 }finally{h.close()}
});

test('route suggestions overlay the form below their search input',async()=>{
 const h=await page('/cars/1'),w=h.w,d=w.document;
 try{
  const style=d.createElement('style');style.textContent=readFileSync(new URL('../style.css',import.meta.url),'utf8');d.head.append(style);
  w.entryForm(1);
  for(const side of ['start','end']){
   const box=d.querySelector('#'+side+'-list'),input=d.querySelector('[name='+side+'_address]');
   assert.equal(box.parentElement,input.parentElement);
   assert.equal(w.getComputedStyle(box).position,'absolute');
   assert.equal(w.getComputedStyle(box.parentElement).position,'relative');
   assert.equal(box.parentElement.contains(d.querySelector('[name='+side+'_address_detail]')),false);
  }
 }finally{h.close()}
});


test('drivers page profile form retains input after a failed save',async()=>{
 const h=await page('/drivers'),w=h.w,d=w.document;
 try{
  assert.equal(d.querySelector('#pageTitle').textContent,'司机管理');
  assert.equal(d.querySelector('aside nav a:last-child').getAttribute('href'),'#/drivers');
  w.driverForm();const f=d.querySelector('#form');f.elements.name.value='张师傅';f.elements.phone.value='13800138000';f.elements.note.value='夜班';
  const boxes=[...f.querySelectorAll('[name=car_ids]')];assert.equal(boxes.length,0);
  let submitted;w.service.request=async(path,options)=>{submitted=JSON.parse(options.body);throw Error('保存失败')};
  await f.onsubmit({preventDefault(){}});assert.equal(submitted.car_ids,undefined);
  assert.equal(f.elements.name.value,'张师傅');assert.equal(d.querySelector('#modal').open,true);
 }finally{h.close()}
});


test('drivers search edit save and delete through normal page controls',async()=>{
 const h=await page('/drivers'),w=h.w,d=w.document;
 try{
  assert.equal(d.querySelectorAll('#driverRows tr').length,2);
  for(const query of ['张师傅','13800138000','测试002','夜班']){
   d.querySelector('#driverSearch').value=query;w.renderDriverRows();
   assert.equal(d.querySelectorAll('#driverRows tr').length,1);assert.match(d.querySelector('#driverRows').textContent,/张师傅/);
  }
  w.driverForm(1);const f=d.querySelector('#form');assert.equal(f.elements.phone.value,'13800138000');assert.equal(f.querySelectorAll('[name=car_ids]').length,0);
  f.elements.phone.value='';f.elements.note.value='白班';f.querySelectorAll('[name=car_ids]').forEach(box=>box.checked=false);
  const data=fixture(),requests=[];
  w.service.request=async(path,options)=>{
   requests.push(path);
   if(path==='/api/drivers'){const record=JSON.parse(options.body);assert.equal(record.id,1);assert.equal(record.phone,'');assert.equal(record.car_ids,undefined);Object.assign(data.drivers[0],record);}
   if(path==='/api/drivers/delete')data.drivers=data.drivers.filter(driver=>driver.id!==JSON.parse(options.body).id);
   return {ok:true,json:async()=>structuredClone(data)};
  };
  await f.onsubmit({preventDefault(){}});assert.equal(d.querySelector('#modal').open,false);assert.match(d.querySelector('#driverRows').textContent,/白班/);
  w.confirmDialog=async()=>false;await w.removeDriver(1);assert.equal(requests.includes('/api/drivers/delete'),false);
  w.confirmDialog=async()=>true;await w.removeDriver(1);assert.doesNotMatch(d.querySelector('#driverRows').textContent,/张师傅/);
 }finally{h.close()}
});


test('vehicles assign drivers and driver forms only edit profile fields',async()=>{
 const h=await page('/cars'),w=h.w,d=w.document;
 try{
  assert.match(d.querySelector('table:has(#cars) th:nth-child(2)').textContent,/司机/);
  w.carForm(1);const f=d.querySelector('#form');assert.equal(f.elements.vin,undefined);
  assert.equal(f.querySelectorAll('[name=driver_ids]:checked').length,1);
  assert.match(f.textContent,/张师傅/);assert.equal(w.frontendDirty(),false);
  w.driverForm(1);assert.equal(f.querySelectorAll('[name=car_ids]').length,0);
 }finally{h.close()}
});


test('driver actions are displayed after vehicles in the final column',async()=>{
 const h=await page('/drivers'),d=h.w.document;
 try{
  assert.deepEqual([...d.querySelectorAll('table th')].map(th=>th.textContent),['司机姓名','手机','车辆','备注','操作']);
  assert.equal(d.querySelector('#driverRows tr td:last-child button').textContent,'编辑');
  assert.match(d.querySelector('#driverRows tr td:nth-child(3)').textContent,/测试001.*测试002/);
 }finally{h.close()}
});

test('car driver selection saves and clears while search totals and export agree',async()=>{
 const h=await page('/cars'),w=h.w,d=w.document;
 try{
  const state=fixture();state.drivers[0].car_ids=[1];
  w.service.request=async(path,options)=>{
   if(path==='/api/cars'){
    const record=JSON.parse(options.body);assert.equal(record.id,2);
    assert.deepEqual(Array.from(record.driver_ids),[1,2]);
    state.drivers.forEach(driver=>driver.car_ids=[...driver.car_ids.filter(id=>id!==2),2]);
   }
   return {ok:true,json:async()=>structuredClone(state)};
  };
  w.carForm(2);const form=d.querySelector('#form');form.querySelectorAll('[name=driver_ids]').forEach(box=>box.checked=true);
  await form.onsubmit({preventDefault(){}});assert.match(d.querySelector('#cars').textContent,/李师傅/);
  w.service.request=async()=>({ok:true,json:async()=>({...fixture(),drivers:[]})});await w.load();
  assert.ok([...d.querySelectorAll('#cars tr')].every(row=>row.cells[1].textContent===''));
  w.service.request=async()=>({ok:true,json:async()=>state});await w.load();
  d.querySelector('#search').value='李师傅';w.render();assert.equal(d.querySelectorAll('#cars tr').length,1);
  assert.equal(d.querySelector('#cars tr a').textContent,'测试002');
  assert.match(d.querySelector('#vehicleOverviewTotals').textContent,/1 辆.*1 笔/);
  await w.exportVehicleOverview();assert.equal(h.calls.at(-1).args.ids.length,1);
 }finally{h.close()}
});

test('address actions use a compact right-aligned desktop column',()=>{
 const css=readFileSync(new URL('../style.css',import.meta.url),'utf8');
 assert.match(css,/table:has\(#addressRows\) th:last-child,table:has\(#addressRows\) td:last-child\{width:1%;white-space:nowrap;text-align:right\}/);
});

test('vehicle and category actions are compact and right aligned',()=>{
 const css=readFileSync(new URL('../style.css',import.meta.url),'utf8');
 assert.ok(css.includes('#cars tr > :nth-child(6), table:has(#cars) thead th:nth-child(6){text-align:right;white-space:nowrap;width:1%}'));
 assert.ok(css.includes('.category-lists th:last-child,.category-lists td:last-child{width:1%;white-space:nowrap;text-align:right}'));
});


test('operation headings stay left aligned within their compact columns',()=>{
 const css=readFileSync(new URL('../style.css',import.meta.url),'utf8');
 assert.ok(css.includes('table:has(#cars) thead th:nth-child(6),table:has(#addressRows) th:last-child,.category-lists th:last-child,.drivers-table th:last-child{text-align:left}'));
});

test('empty counterparties render as blank ledger cells',async()=>{
 for(const route of ['/entries','/cars/1']){
  const h=await page(route),w=h.w,d=w.document;
  try{
   const state=fixture();state.entries.forEach(entry=>entry.party='');
   w.service.request=async()=>({ok:true,json:async()=>state});await w.load();
   assert.ok([...d.querySelectorAll('#vehicleEntries td[data-label="往来方"]')].every(cell=>cell.textContent===''));
  }finally{h.close()}
 }
});

test('driver header filter includes unassigned and updates rows totals export and clear',async()=>{
 const h=await page('/cars'),w=h.w,d=w.document;
 try{
  assert.match(d.querySelector('table:has(#cars) th:nth-child(2)').textContent,/司机.*▾/);
  assert.match(d.querySelector('#vehicleDriverChoices').textContent,/未分配司机/);
  w.toggleAllVehicleDrivers();assert.match(d.querySelector('#vehicleOverviewTotals').textContent,/0 辆/);
  w.toggleVehicleDriver({value:'1',checked:true});assert.equal(d.querySelectorAll('#cars tr').length,2);
  d.querySelector('#vehicleDriverSearch').value='李';w.renderVehicleDriverFilter();assert.doesNotMatch(d.querySelector('#vehicleDriverChoices').textContent,/张师傅/);
  w.toggleAllVehicleDrivers();w.toggleAllVehicleDrivers();w.toggleVehicleDriver({value:'unassigned',checked:true});
  assert.match(d.querySelector('#vehicleOverviewTotals').textContent,/0 辆/);
  w.clearFilters();assert.equal(d.querySelectorAll('#cars tr').length,2);
  const state=fixture();state.drivers[0].car_ids=[1];w.service.request=async()=>({ok:true,json:async()=>state});await w.load();
  w.toggleAllVehicleDrivers();w.toggleVehicleDriver({value:'unassigned',checked:true});
  assert.equal(d.querySelector('#cars tr a').textContent,'测试002');
  await w.exportVehicleOverview();assert.equal(h.calls.at(-1).args.ids.length,1);
 }finally{h.close()}
});

test('custom addresses have a separate searchable CRUD list and editable form',async()=>{
 const h=await page('/addresses'),w=h.w,d=w.document;
 try{
  assert.equal(d.querySelectorAll('#customAddressRows tr').length,2);
  assert.doesNotMatch(d.querySelector('#addressRows').textContent,/上海/);
  d.querySelector('#customAddressSearch').value='杭州';w.renderCustomAddressRows();
  assert.doesNotMatch(d.querySelector('#customAddressRows').textContent,/上海/);
  w.customAddressForm(2);const form=d.querySelector('#form');assert.equal(form.elements.name.value,'杭州');
  form.elements.detail.value='小区602室';let record;
  w.service.request=async(path,options)=>{record=JSON.parse(options.body);throw Error('保存失败')};
  await form.onsubmit({preventDefault(){}});assert.equal(record.id,2);assert.equal(record.detail,'小区602室');assert.equal(form.elements.detail.value,'小区602室');
  w.customAddressForm();assert.equal(form.elements.name.value,'');
 }finally{h.close()}
});

test('custom addresses save delete and become route suggestions',async()=>{
 const h=await page('/addresses'),w=h.w,d=w.document,state=fixture();
 try{
  w.service.request=async(path,options={})=>{
   if(path==='/api/addresses'){const record=JSON.parse(options.body);state.addresses.push({id:3,active:1,...record});}
   if(path==='/api/addresses/delete')state.addresses.find(address=>address.id===JSON.parse(options.body).id).active=0;
   return {ok:true,json:async()=>structuredClone(state)};
  };
  w.customAddressForm();let form=d.querySelector('#form');form.elements.name.value='西湖仓库';form.elements.detail.value='8号门';
  await form.onsubmit({preventDefault(){}});assert.match(d.querySelector('#customAddressRows').textContent,/西湖仓库.*8号门/);
  w.entryForm(1);form=d.querySelector('#form');form.elements.start_address.value='xihu';w.suggest('start');
  assert.match(d.querySelector('#start-list').textContent,/西湖仓库/);
  await w.removeAddress(3);assert.doesNotMatch(d.querySelector('#customAddressRows').textContent,/西湖仓库/);
 }finally{h.close()}
});

test('entry deletion requires confirmation retains failed input and refreshes after success',async()=>{
 for(const route of ['/entries','/cars/1']){
  const h=await page(route),w=h.w,d=w.document,state=fixture(),requests=[];
  try{
   w.entryForm(1);assert.equal(d.querySelector('#deleteRecord').hidden,true);
   w.editLedgerEntry(2);assert.equal(d.querySelector('#deleteRecord').hidden,false);
   d.querySelector('#form').elements.note.value='尚未保存';
   w.service.request=async(path,options)=>{requests.push(path);return {ok:true,json:async()=>state}};
   w.confirmDialog=async message=>{assert.match(message,/测试001/);assert.match(message,/100\.00/);return false};
   await w.deleteEditedRecord();assert.equal(requests.length,0);assert.equal(d.querySelector('#form').elements.note.value,'尚未保存');
   w.confirmDialog=async()=>true;w.service.request=async()=>{throw Error('删除失败')};
   await w.deleteEditedRecord();assert.equal(d.querySelector('#modal').open,true);assert.equal(d.querySelector('#save').disabled,false);assert.equal(d.querySelector('#form').elements.note.value,'尚未保存');
   let finish;
   w.service.request=async(path,options)=>{
    requests.push(path);
    if(path==='/api/entries/delete'){assert.equal(JSON.parse(options.body).id,2);await new Promise(resolve=>finish=resolve);state.entries=state.entries.filter(entry=>entry.id!==2);}
    return {ok:true,json:async()=>state};
   };
   const deleting=w.deleteEditedRecord();await new Promise(resolve=>setTimeout(resolve,0));assert.equal(w.frontendBusy(),true);await w.deleteEditedRecord();
   finish();await deleting;assert.equal(requests.filter(path=>path==='/api/entries/delete').length,1);assert.equal(d.querySelector('#modal').open,false);assert.equal(w.frontendBusy(),false);
   w.driverForm(1);assert.equal(d.querySelector('#deleteRecord').hidden,true);
  }finally{h.close()}
 }
});


test('vehicle deletion blocks recorded vehicles and confirms empty vehicles',async()=>{
 const h=await page('/cars'),w=h.w,d=w.document,state=fixture();
 try{
  let requests=0,confirmations=0;
  w.service.request=async(path,options)=>{requests++;return {ok:true,json:async()=>state}};
  w.confirmDialog=async()=>{confirmations++;return true};w.carForm(1);
  assert.equal(d.querySelector('#deleteRecord').hidden,false);await w.deleteEditedRecord();
  assert.match(d.querySelector('#error').textContent,/收支记录/);assert.equal(requests,0);assert.equal(confirmations,0);
  state.entries=[];w.service.request=async()=>({ok:true,json:async()=>state});await w.load();w.carForm(1);
  w.confirmDialog=async message=>{assert.match(message,/测试001/);return false};await w.deleteEditedRecord();assert.equal(d.querySelector('#modal').open,true);
  w.confirmDialog=async()=>true;w.service.request=async(path,options)=>{
   if(path==='/api/cars/delete'){assert.equal(JSON.parse(options.body).id,1);state.cars=state.cars.filter(car=>car.id!==1);state.drivers.forEach(driver=>driver.car_ids=driver.car_ids.filter(id=>id!==1));}
   return {ok:true,json:async()=>state};
  };
  await w.deleteEditedRecord();assert.equal(d.querySelector('#modal').open,false);assert.doesNotMatch(d.querySelector('#cars').textContent,/测试001/);
  w.carForm();assert.equal(d.querySelector('#deleteRecord').hidden,true);
 }finally{h.close()}
});

test('entry calendar changes the form date without changing list filters or losing edits',async()=>{
 const h=await page('/entries'),w=h.w,d=w.document;
 try{
  w.editLedgerEntry(1);
  const f=d.querySelector('#form'),date=f.elements.date;
  f.elements.note.value='保留这条备注';
  date.click();
  const panel=d.querySelector('#ledgerCalendar');
  assert.equal(panel.matches(':popover-open'),true);
  assert.equal(panel.closest('dialog'),d.querySelector('#modal'));
  assert.equal(d.querySelector('#calendarMonthTitle').textContent,'2026年10月');
  assert.equal(d.querySelector('.calendar-day.selected').getAttribute('aria-label').startsWith('2026-10-25'),true);
  w.changeCalendarMonth(1);w.chooseCalendarDay('2026-11-03');
  assert.equal(date.value,'2026-11-03');
  assert.equal(f.elements.note.value,'保留这条备注');
  assert.equal(d.querySelector('#start').value,'');
  assert.equal(d.querySelector('#end').value,'');
  assert.match(d.querySelector('#subtotal').textContent,/81 笔/);
  assert.equal(d.activeElement,date);
  date.click();w.chooseCalendarDay('');
  await f.onsubmit({preventDefault(){}});
  assert.match(d.querySelector('#error-date').textContent,/请填写/);
  assert.equal(d.querySelector('#modal').open,true);
  w.entryForm();f.elements.date.click();
  assert.equal(panel.matches(':popover-open'),true);
  await w.requestClose();
  assert.equal(panel.matches(':popover-open'),false);
  d.querySelector('#start').click();w.chooseCalendarDay('2026-10-31');
  assert.equal(panel.parentElement,d.body);
  assert.match(d.querySelector('#subtotal').textContent,/11 笔/);
 }finally{h.close()}
});
