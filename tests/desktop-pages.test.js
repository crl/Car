import test from 'node:test';
import assert from 'node:assert/strict';
import {readFileSync} from 'node:fs';
import vm from 'node:vm';
import {JSDOM} from 'jsdom';
const html=readFileSync(new URL('../index.html',import.meta.url),'utf8');
const source=readFileSync(new URL('../app.js',import.meta.url),'utf8').replace(/^import .*;$/gm,'');
function fixture(){return {cars:[{id:1,plate:'测试001',name:'旧车型',vin:'VIN1',note:'备注'},{id:2,plate:'测试002',name:'',vin:'',note:''}],categories:[{id:1,kind:'收入',name:'运费',active:1,requires_route:1},{id:2,kind:'支出',name:'油费',active:1,requires_route:0}],addresses:[{id:1,name:'上海',detail:'',active:1},{id:2,name:'杭州',detail:'',active:1}],entries:Array.from({length:81},(_,i)=>({id:i+1,car_id:i===80?2:1,date:'2026-10-'+String(25+i%7).padStart(2,'0'),kind:i%2?'支出':'收入',category:i%2?'油费':'运费',cents:10000,start_address_id:i%3?1:null,end_address_id:i%3?2:null,start_address:i%3?'上海':null,end_address:i%3?'杭州':null,party:i%2?'油站':'客户',note:'备注'}))}}
async function page(route){const dom=new JSDOM(html,{url:'http://tauri.localhost/#'+route,runScripts:'outside-only'}),w=dom.window,calls=[],data=fixture();
 w.service={request:async()=>({ok:true,json:async()=>structuredClone(data)}),call:async(cmd,args)=>{calls.push({cmd,args});return {cancelled:true}}};w.confirmDialog=async()=>true;w.alertDialog=async()=>{};
 w.HTMLDialogElement.prototype.showModal=function(){this.open=true};w.HTMLDialogElement.prototype.close=function(){this.open=false};
 const matches=w.Element.prototype.matches;w.Element.prototype.matches=function(s){return s===':popover-open'?this.hasAttribute('data-open'):matches.call(this,s)};
 w.HTMLElement.prototype.showPopover=function(){this.setAttribute('data-open','')};w.HTMLElement.prototype.hidePopover=function(){this.removeAttribute('data-open')};
 await vm.runInContext('(async()=>{'+source+'})()',dom.getInternalVMContext());return {dom,w,calls,close:()=>w.close()};}
test('all desktop routes render and expose inline handlers',async()=>{for(const route of ['/cars','/entries','/cars/1','/categories','/addresses','/cars/999']){const h=await page(route);try{assert.doesNotMatch(h.w.document.querySelector('main').textContent,/读取失败/);assert.equal(typeof h.w.entryForm,'function');assert.equal(typeof h.w.openSiteSettings,'function');if(route==='/cars/999')assert.match(h.w.document.querySelector('main').textContent,/车辆不存在/)}finally{h.close()}}});
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
