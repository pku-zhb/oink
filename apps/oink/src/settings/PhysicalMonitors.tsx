import { useCallback, useEffect, useRef, useState } from 'react';
import { Button, Group, Select, Switch } from 'oink-ui';
import { api } from '../api/client';
import { agent, useAgent } from '../api/store';
import { busyOperation, physicalDesktop, presets, sameChoice } from '../api/physicalDesktop';
import type { DesktopMode, DesktopState } from '../api/physicalDesktop';
import type { ScreenKey } from '../api/types';
import { PhysicalSetup } from './PhysicalSetup';
import { tr } from '../i18n';
import './physical-monitors.css';

export function PhysicalMonitors() {
  const { overview, cards } = useAgent();
  const [state, setState] = useState<DesktopState | null>(null);
  const [error, setError] = useState('');
  const [editing, setEditing] = useState(false);
  const [sending, setSending] = useState(false);
  const [mode, setMode] = useState<DesktopMode>('compatible');
  const [baseline, setBaseline] = useState(false);
  const [selected, setSelected] = useState<ScreenKey[]>([]);
  const [now, setNow] = useState(Date.now());
  const mounted = useRef(true);
  const names = new Map((cards?.cards ?? []).map(d => [d.device,d.name]));
  if (overview) names.set(overview.this_device.id,overview.this_device.name);
  const name = (id: string) => names.get(id) ?? id.slice(0,8);
  const refresh = useCallback(async () => {
    const next = await physicalDesktop({action:'status'});
    if (mounted.current) setState(next);
    return next;
  },[]);
  useEffect(() => {
    mounted.current = true;
    let pending = false;
    const poll = async () => {
      if (pending) return;
      pending = true;
      try { await refresh(); } catch (e) { if (mounted.current) setError(e instanceof Error ? e.message : String(e)); }
      finally { pending = false; }
    };
    void poll();
    const timer = window.setInterval(() => { setNow(Date.now()); void poll(); },500);
    return () => { mounted.current = false; window.clearInterval(timer); };
  },[refresh]);
  useEffect(() => {
    if (!state?.mapping) return;
    setSelected(current => current.length === state.mapping!.panels.length ? current : state.active.length ? state.active : state.mapping!.panels.map(p => (p.endpoints.find(e=>e.screen.device===p.baseline) ?? p.endpoints[0]).screen));
  },[state?.mapping?.revision]);
  const run = async (work: () => Promise<unknown>) => {
    if (sending) return;
    setSending(true);setError('');
    try { await work(); await refresh(); await agent.query('layout',api.layout); }
    catch (e) { if (mounted.current) setError(e instanceof Error ? e.message : String(e)); }
    finally { if (mounted.current) setSending(false); }
  };
  const busy = busyOperation(state?.operation ?? null);
  const waiting = state?.operation?.phase === 'awaiting_confirmation';
  const cooling = (state?.cooldown_until ?? 0) > now;
  const needsBaseline = !!state?.mapping?.panels.some(p=>!p.readback) && (state?.baseline_required ?? !state?.active.length) && !baseline;
  const cannotApply = busy || sending || cooling || needsBaseline || state?.operation?.phase==='needs_attention' || !!state?.coordinator_error;
  const seconds = Math.max(0,Math.ceil(((state?.operation?.deadline_ms ?? now)-now)/1000));
  const apply = (choices: ScreenKey[]) => run(() => physicalDesktop({action:'apply',expected_revision:state!.mapping!.revision,choices,mode,baseline_confirmed:baseline}));
  if (!state) return <div className="physical"><p role="status">{error || tr('正在读取共用显示器配置')}</p></div>;
  if (editing) return <PhysicalSetup state={state} thisDevice={overview?.this_device.id ?? ''} name={name}
    onCancel={() => setEditing(false)} onSaved={() => {setEditing(false);setBaseline(true);void refresh();}} onRefresh={refresh}/>;
  return <section className="physical" aria-label={tr('共用显示器')} aria-busy={sending}>
    <div className="physical__intro"><h3>{tr('让每块屏幕显示你需要的电脑')}</h3><p>{tr('确认一次屏幕位置和连接关系，之后直接选择布局。')}</p></div>
    {error && <p role="alert" className="physical__error">{error}</p>}
    {state.coordinator_error && <p role="alert">{tr('协调电脑暂时离线，已保存的绑定仍然保留。')}</p>}
    {!state.mapping ? <>
      <Button variant="accent" onClick={() => setEditing(true)}>{tr('设置共用显示器')}</Button>
      {state.imported && <p className="setp__note">{tr('发现已有绑定，可以在向导中继续使用。')}</p>}
    </> : <>
      <div className="physical__scenes" aria-label={tr('物理屏幕组合')}>
        {presets(state.mapping.panels).map((choices,index) => <button type="button" className="physical__scene" key={index}
          aria-label={choices.map(s=>name(s.device)).join(' / ')} aria-pressed={choices.length===state.active.length&&choices.every((s,i)=>sameChoice(s,state.active[i]))}
          disabled={cannotApply} onClick={() => {setSelected(choices);void apply(choices);}}>
          <span className="physical__scene-monitors">{choices.map((choice,i)=><span className="physical__mini-monitor" key={i}><span>{name(choice.device)}</span><small>{state.mapping!.panels[i].name}</small></span>)}</span>
        </button>)}
      </div>
      <fieldset className="physical__setup" disabled={busy||sending||!!state.coordinator_error}>
        <legend>{tr('逐屏选择电脑')}</legend>
        <div className="physical__selectors">{state.mapping.panels.map((panel,i)=><label key={panel.id}>{panel.name}
          <Select aria-label={panel.name} value={selected[i]?.device ?? panel.baseline} options={panel.endpoints.map(e=>({value:e.screen.device,label:name(e.screen.device)}))}
            onChange={device=>setSelected(old=>state.mapping!.panels.map((p,j)=>j===i?p.endpoints.find(e=>e.screen.device===device)!.screen:old[j]))}/>
        </label>)}</div>
        <Button variant="accent" disabled={cannotApply||selected.length!==state.mapping.panels.length} onClick={()=>{void apply(selected);}}>{tr('应用布局')}</Button>
        <label className="physical__switch"><Switch aria-label={tr('同时停用不可见的系统显示器')} checked={mode==='complete'} onChange={enabled=>setMode(enabled?'complete':'compatible')}/>{tr('同时停用不可见的系统显示器')}</label>
        <p className="setp__note">{mode==='complete'?tr('将同步系统桌面和主屏。平台不支持时会恢复原配置。'):tr('兼容模式会保留系统扩展桌面，窗口仍可能出现在不可见屏幕。')}</p>
        {(state.baseline_required??!state.active.length)&&state.mapping.panels.some(p=>!p.readback)&&<label className="physical__switch"><Switch aria-label={tr('当前画面与设置时确认的电脑一致')} checked={baseline} onChange={setBaseline}/>{tr('当前画面与设置时确认的电脑一致')}</label>}
      </fieldset>
    </>}
    {cooling&&!busy&&<p className="setp__note" role="status">{tr('请等待显示器稳定后再切换')}</p>}
    {state.operation&&<div className={`physical__operation physical__operation--${state.operation.phase}`} role={waiting?'alert':'status'}>
      <strong>{waiting?tr('{0} 秒后恢复原布局',seconds):phaseText(state.operation.phase)}</strong>
      {busy&&!waiting&&<p>{tr('正在协调显示器，请等待完成。')}</p>}
      {waiting&&<Group><Button variant="accent" disabled={sending||seconds===0} onClick={()=>{void run(()=>physicalDesktop({action:'keep',operation:state.operation!.id}));}}>{tr('保留布局')}</Button>
        <Button disabled={sending} onClick={()=>{void run(()=>physicalDesktop({action:'revert',operation:state.operation!.id}));}}>{tr('恢复原布局')}</Button></Group>}
      {state.operation.phase==='completed'&&!state.operation.hardware_verified&&<p>{tr('已按校准映射切换，部分显示器不能报告实际输入源。')}</p>}
      {['failed','needs_attention','reverted'].includes(state.operation.phase)&&state.operation.detail&&<details><summary>{tr('查看详情')}</summary><p>{state.operation.detail}</p></details>}
    </div>}
    {state.operation?.phase==='needs_attention'&&!state.has_entry&&state.operation.mode===null&&<Button disabled={sending} onClick={()=>{void run(()=>physicalDesktop({action:'recover_calibration',operation:state.operation!.id}));}}>{tr('恢复测试前输入')}</Button>}
    {state.mapping&&<Group className="physical__actions">
      <Button disabled={busy||sending||state.has_entry||state.active.length>0} onClick={()=>setEditing(true)}>{tr('重新配置屏幕')}</Button>
      {(state.has_entry||state.active.length>0)&&<Button disabled={busy||sending} onClick={()=>{void run(()=>physicalDesktop({action:'restore'}));}}>{tr('恢复切换前配置')}</Button>}
      {state.coordinator_error&&state.has_entry&&<Button disabled={sending} onClick={()=>{void run(()=>physicalDesktop({action:'recover_local'}));}}>{tr('仅恢复这台电脑')}</Button>}
    </Group>}
  </section>;
}
export function phaseText(phase: string): string {
  switch(phase) {
    case 'checking': return tr('正在检查连接');
    case 'preparing': return tr('正在保存恢复点');
    case 'connecting': return tr('正在重新连接显示器');
    case 'switching': return tr('正在切换输入源');
    case 'desktop': return tr('正在更新桌面和主屏');
    case 'restoring': return tr('正在恢复原配置');
    case 'completed': return tr('布局已应用');
    case 'reverted': return tr('已恢复上一配置');
    case 'needs_attention': return tr('部分配置需要恢复');
    case 'calibrating': return tr('正在测试，随后自动恢复');
    case 'calibration_ready': return tr('测试结束，已恢复原输入');
    default: return tr('操作未完成');
  }
}
