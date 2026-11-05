import { useState } from 'react';
import { Button, Group, Input, Select, Switch } from 'oink-ui';
import { api } from '../api/client';
import { useAgent } from '../api/store';
import { busyOperation, physicalDesktop, suggestPanels } from '../api/physicalDesktop';
import type { DesktopState, Endpoint, Inspection, Mapping, Panel } from '../api/physicalDesktop';
import { tr } from '../i18n';

const PORTS = [
  {value:'17',label:'HDMI 1'},{value:'18',label:'HDMI 2'},
  {value:'15',label:'DisplayPort 1'},{value:'16',label:'DisplayPort 2'},
  {value:'3',label:'DVI 1'},{value:'4',label:'DVI 2'},
];
interface Props {
  state: DesktopState; thisDevice: string; name: (device: string) => string;
  onCancel: () => void; onSaved: () => void; onRefresh: () => Promise<DesktopState>;
}
export function PhysicalSetup({state,thisDevice,name,onCancel,onSaved,onRefresh}: Props) {
  const { cards } = useAgent();
  const [step,setStep] = useState(0);
  const [draft,setDraft] = useState<Mapping>(() => structuredClone(state.mapping ?? state.imported ?? {revision:0,group:state.group,coordinator:thisDevice,panels:[]}));
  const root = state.mapping?.coordinator ?? thisDevice;
  const devices = [...new Set([root,...(cards?.cards ?? []).map(c=>c.device)])];
  const [partner,setPartner] = useState(draft.panels.flatMap(p=>p.endpoints).find(e=>e.screen.device!==root)?.screen.device ?? devices.find(d=>d!==root) ?? '');
  const [inspections,setInspections] = useState<Inspection[]>([]);
  const [working,setWorking] = useState(false);
  const [error,setError] = useState('');
  const [testing,setTesting] = useState<{panel:string;device:string;operation:string} | null>(null);
  const [confirmed,setConfirmed] = useState<Record<string,boolean>>({});
  const [advanced,setAdvanced] = useState(false);
  const [rawInputs,setRawInputs] = useState<Record<string,string>>({});
  const busy = working || busyOperation(state.operation);
  const tasks = [tr('连接电脑'),tr('摆放位置'),tr('输入口匹配'),tr('确认设置')];
  const run = async (work: () => Promise<unknown>) => {
    if (working) return;
    setWorking(true);setError('');
    try { await work(); } catch(e) {setError(e instanceof Error ? e.message : String(e));}
    finally {setWorking(false);}
  };
  const invalidate = (id: string) => {setConfirmed(old=>Object.fromEntries(Object.entries(old).filter(([key])=>!key.startsWith(`${id}:`))));setTesting(null);};
  const patch = (id: string,change: Partial<Panel>,reset=true) => {
    if (reset) invalidate(id);
    setDraft(old=>({...old,panels:old.panels.map(p=>p.id===id?{...p,...change,...(reset?{calibrated:false}:{})}:p)}));
  };
  const endpoint = (panel: Panel,device: string,change: Partial<Endpoint>,raw?:string) => {
    if (change.input!==undefined) setRawInputs(old=>({...old,[`${panel.id}:${device}`]:raw??(change.input?String(change.input):'')}));
    const old = panel.endpoints.find(e=>e.screen.device===device);
    const next = {...(old ?? {screen:{device,display:''},input:0,port:''}),...change};
    patch(panel.id,{endpoints:[...panel.endpoints.filter(e=>e.screen.device!==device),next]});
  };
  const inspect = () => run(async () => {
    const results = await Promise.all([root,partner].map(device=>physicalDesktop<Inspection>({action:'inspect',device})));
    if (results.some(i=>!i.ddc_supported)) throw new Error(tr('有电脑尚不支持显示器输入控制，请先完成安装和权限设置。'));
    setInspections(results);
    setDraft(old=>({...old,coordinator:root,panels:old.panels.length?old.panels:suggestPanels(results[0],results.slice(1))}));
    setStep(1);
  });
  const valid = draft.panels.length>0&&draft.panels.every(p=>p.endpoints.length===2&&[root,partner].every(d=>p.endpoints.some(e=>e.screen.device===d&&e.screen.display&&e.input>0))&&p.endpoints.some(e=>e.screen.device===p.baseline));
  const tested = testing && state.operation?.id===testing.operation;
  const resultReady = tested && state.operation?.phase==='calibration_ready';
  const confirmTest = () => {
    if (!testing) return;
    const next = {...confirmed,[`${testing.panel}:${testing.device}`]:true};
    setConfirmed(next);
    const panel = draft.panels.find(p=>p.id===testing.panel)!;
    patch(panel.id,{calibrated:panel.endpoints.every(e=>next[`${panel.id}:${e.screen.device}`])},false);
    setTesting(null);
  };
  return <section className="physical physical__wizard" aria-label={tr('设置共用显示器')}>
    <div className="physical__wizard-header"><p className="setp__note">{tr('第 {0} 步，共 {1} 步',step+1,tasks.length)}</p><h3>{tasks[step]}</h3></div>
    <ol className="physical__steps" aria-label={tr('设置进度')}>{tasks.map((task,i)=><li key={task} aria-current={i===step?'step':undefined} data-complete={i<step?'':undefined}>{task}</li>)}</ol>
    {error&&<p role="alert" className="physical__error">{error}</p>}
    {step===0&&<>
      <p>{tr('两台电脑都连接目标显示器，并保持在线。设置完成后会自动共享绑定。')}</p>
      <p>{tr('协调电脑：{0}',name(root))}</p>
      <label>{tr('另一台电脑')}<Select aria-label={tr('另一台电脑')} value={partner} options={[{value:'',label:tr('选择电脑')},...devices.filter(d=>d!==root).map(d=>({value:d,label:name(d)}))]} onChange={value=>{setPartner(value);setDraft(old=>({...old,panels:[]}));}}/></label>
      {state.imported&&!state.mapping&&<Button onClick={()=>{
        const imported=structuredClone(state.imported!);imported.coordinator=root;
        setDraft(imported);setConfirmed({});
      }}>{tr('从现有绑定开始')}</Button>}
      <Button variant="accent" disabled={!partner||working} onClick={()=>{void inspect();}}>{working?tr('正在检查连接'):tr('检查并继续')}</Button>
    </>}
    {step===1&&<>
      <p>{tr('按桌上真实位置从左到右排列。系统里的顺序可能不同。')}</p>
      <Button disabled={busy} onClick={()=>{void run(()=>api.identifyScreens());}}>{tr('识别屏幕')}</Button>
      <div className="physical__position">{draft.panels.map((panel,i)=><div className="physical__monitor" key={panel.id}>
        <span className="physical__number">{i+1}</span><Button size="sm" disabled={busy} onClick={()=>{const screen=panel.endpoints.find(e=>e.screen.device===root)?.screen;if(screen)void run(()=>physicalDesktop({action:'identify',device:root,display:screen.display,label:tr('物理屏幕 {0}',i+1)}));}}>{tr('识别屏幕')}</Button><label>{tr('屏幕名称')}<Input value={panel.name} onChange={value=>patch(panel.id,{name:value},false)}/></label>
        <Group><Button size="sm" disabled={busy||i===0} onClick={()=>setDraft(old=>{const panels=[...old.panels];[panels[i-1],panels[i]]=[panels[i],panels[i-1]];return {...old,panels};})}>{tr('向左移动')}</Button>
          <Button size="sm" disabled={busy||i===draft.panels.length-1} onClick={()=>setDraft(old=>{const panels=[...old.panels];[panels[i+1],panels[i]]=[panels[i],panels[i+1]];return {...old,panels};})}>{tr('向右移动')}</Button></Group>
      </div>)}</div>
      {!draft.panels.length&&<p role="alert">{tr('未发现可配置的外接显示器。')}</p>}
    </>}
    {step===2&&<>
      <p>{tr('为每块屏幕匹配电脑和视频输入口。候选端口必须通过测试确认。')}</p>
      <label className="physical__switch"><Switch aria-label={tr('显示高级输入设置')} checked={advanced} onChange={setAdvanced}/>{tr('显示高级输入设置')}</label>
      <fieldset className="physical__setup" disabled={busy}>
        {draft.panels.map(panel=><div className="physical__monitor" key={panel.id}>
          <h4>{panel.name}</h4>
          <label>{tr('当前实际显示的电脑')}<Select aria-label={`${panel.name} · ${tr('当前实际显示的电脑')}`} value={panel.baseline} options={[root,partner].map(d=>({value:d,label:name(d)}))} onChange={value=>patch(panel.id,{baseline:value})}/></label>
          {[root,partner].map(device=>{
            const binding=panel.endpoints.find(e=>e.screen.device===device);
            const inspection=inspections.find(i=>i.device===device);
            const custom=binding?.input&&!PORTS.some(p=>Number(p.value)===binding.input)?[{value:String(binding.input),label:tr('已保存的输入口')}]:[];
            return <div className="physical__binding" key={device}>
              <label>{name(device)}<Select aria-label={`${panel.name} · ${name(device)} · ${tr('选择屏幕')}`} value={binding?.screen.display??''}
                options={[{value:'',label:tr('选择屏幕')},...(inspection?.displays??[]).filter(d=>!d.is_virtual&&!d.is_builtin).map((d,i)=>({value:d.key,label:`${d.name} · ${i+1}`}))]}
                onChange={display=>endpoint(panel,device,{screen:{device,display}})}/></label>
              <label>{tr('视频输入口')}<Select aria-label={`${panel.name} · ${name(device)} · ${tr('视频输入口')}`} value={String(binding?.input||'')} options={[{value:'',label:tr('选择输入口')},...PORTS,...custom]}
                onChange={value=>endpoint(panel,device,{input:Number(value),port:PORTS.find(p=>p.value===value)?.label??''})}/></label>
              {advanced&&<label>{tr('输入源数值')}<Input aria-label={`${panel.name} · ${name(device)} · ${tr('输入源数值')}`} value={rawInputs[`${panel.id}:${device}`]??(binding?.input?String(binding.input):'')} inputMode="text" onChange={value=>{const input=Number(value);endpoint(panel,device,{input:Number.isInteger(input)&&input>0&&input<=65535?input:0},value);}}/></label>}
              <Group className="physical__actions">
                <Button size="sm" disabled={!binding?.screen.display} onClick={()=>{void run(async()=>{
                  const result=await physicalDesktop<{input:number}>({action:'read',device,display:binding!.screen.display});
                  endpoint(panel,device,{input:result.input});patch(panel.id,{readback:true},false);
                });}}>{tr('读取当前输入')}</Button>
                <Button size="sm" disabled={!binding?.input||!binding.screen.display||state.cooldown_until>Date.now()} onClick={()=>{void run(async()=>{
                  const result=await physicalDesktop({action:'calibrate',panel,target:device});
                  if(result.operation)setTesting({panel:panel.id,device,operation:result.operation.id});
                  await onRefresh();
                });}}>{tr('测试并恢复')}</Button>
                {confirmed[`${panel.id}:${device}`]&&<span className="physical__verified">{tr('已确认')}</span>}
              </Group>
            </div>;
          })}
          {advanced&&<label className="physical__switch"><Switch aria-label={`${panel.name} · ${tr('验证输入源读回')}`} checked={panel.readback} onChange={readback=>patch(panel.id,{readback})}/>{tr('验证输入源读回')}</label>}
        </div>)}
      </fieldset>
      <p className="setp__note">{tr('读取输入前，请先让屏幕显示所选电脑。不能读取的屏幕使用目视校准和已确认的起始画面。')}</p>
      {state.imported&&!state.mapping&&valid&&<label className="physical__switch"><Switch aria-label={tr('这些现有绑定已通过我的日常切换验证')} checked={draft.panels.every(p=>p.calibrated)} onChange={value=>setDraft(old=>({...old,panels:old.panels.map(p=>({...p,calibrated:value}))}))}/>{tr('这些现有绑定已通过我的日常切换验证')}</label>}
      {tested&&busy&&<div className="physical__operation" role="status"><p>{tr('测试后会自动恢复原输入，请不要重复切换。')}</p>
        {state.operation?.phase==='calibrating'&&<Button onClick={()=>{void run(()=>physicalDesktop({action:'revert',operation:testing!.operation}));}}>{tr('取消测试并恢复')}</Button>}
      </div>}
      {resultReady&&<div className="physical__operation" role="alert"><p>{tr('已恢复原画面。刚才是否显示了 {0}？',name(testing!.device))}</p><Group><Button variant="accent" onClick={confirmTest}>{tr('显示正确')}</Button><Button onClick={()=>setTesting(null)}>{tr('未显示，重新选择')}</Button></Group></div>}
      {tested&&['failed','needs_attention'].includes(state.operation?.phase??'')&&<div role="alert"><p>{state.operation?.detail}</p>{state.operation?.phase==='needs_attention'&&<Button onClick={()=>{void run(async()=>{await physicalDesktop({action:'recover_calibration',operation:testing!.operation});await onRefresh();});}}>{tr('恢复测试前输入')}</Button>}</div>}
    </>}
    {step===3&&<>
      <p>{tr('绑定将共享到参与电脑。首次切换系统桌面时仍会验证实际结果，并提供恢复倒计时。')}</p>
      <div className="physical__monitor">{draft.panels.map((panel,i)=><p key={panel.id}><strong>{i+1}. {panel.name}</strong> · {panel.endpoints.map(e=>name(e.screen.device)).join(' / ')} · {panel.readback?tr('支持读回验证'):tr('使用已确认的校准映射')}</p>)}</div>
      {inspections.map(i=><p className="setp__note" key={i.device}>{name(i.device)} · {i.desktop_supported?tr('系统桌面适配可用，尚待实机验证'):tr('系统停屏不可用，可使用兼容模式')}</p>)}
      <Button variant="accent" disabled={busy||!valid||draft.panels.some(p=>!p.calibrated)} onClick={()=>{void run(async()=>{
        await physicalDesktop({action:'save',expected_revision:state.mapping?.revision??0,mapping:draft});onSaved();
      });}}>{tr('保存并完成')}</Button>
    </>}
    <Group className="physical__actions">
      <Button disabled={busy} onClick={onCancel}>{tr('取消')}</Button>
      {step>0&&<Button disabled={busy} onClick={()=>setStep(s=>s-1)}>{tr('上一步')}</Button>}
      {step>0&&step<3&&<Button variant="accent" disabled={busy||!draft.panels.length||(step===2&&(!valid||draft.panels.some(p=>!p.calibrated)))} onClick={()=>setStep(s=>s+1)}>{tr('继续')}</Button>}
    </Group>
  </section>;
}
