import { useEffect, useState } from 'react';
import { Button, Group, Input, Select } from 'oink-ui';
import { api } from '../api/client';
import { physicalCall, monitorScenes, sameScreen } from '../api/physicalMonitors';
import type { PhysicalMonitor, PhysicalState } from '../api/physicalMonitors';
import type { ScreenKey } from '../api/types';
import { agent, useAgent } from '../api/store';
import { tr } from '../i18n';
import './physical-monitors.css';

export function PhysicalMonitors() {
  const { layout, overview, cards } = useAgent();
  const [state, setState] = useState<PhysicalState | null>(null);
  const [draft, setDraft] = useState<PhysicalMonitor[]>([]);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [notice, setNotice] = useState('');
  const [inputTexts, setInputTexts] = useState<Record<string,string>>({});
  const screens = layout?.geometry.screens ?? [];
  const local = screens.filter(s => s.key.device === overview?.this_device.id && !s.display.is_virtual);
  const computers = Array.from(new Map([
    ...(cards?.cards ?? []).map(m => [m.device, m.name] as const),
    ...screens.map(s => [s.key.device, s.device_name] as const),
  ]).entries());
  const name = (key: ScreenKey) => screens.find(s => s.key.device === key.device)?.device_name
    ?? computers.find(([id]) => id === key.device)?.[1] ?? key.device.slice(0, 8);
  const accept = (next: PhysicalState) => { setState(next); setDraft(next.setup.monitors); setInputTexts({}); };
  useEffect(() => { let live = true; physicalCall({action:'query'}).then(s => { if (live) accept(s); }).catch(e => { if (live) setError(String(e.message)); }); return () => { live = false; }; }, []);
  const run = async (work: () => Promise<void>) => {
    if (busy) return;
    setBusy(true); setError(''); setNotice('');
    try { await work(); } catch (e) { setError(e instanceof Error ? e.message : String(e)); }
    finally { setBusy(false); }
  };
  const update = (index: number, patch: Partial<PhysicalMonitor>) => setDraft(current => current.map((m, i) => i === index ? {...m, ...patch} : m));
  const refreshLayout = () => agent.query('layout', api.layout);
  const apply = (selected: ScreenKey[]) => run(async () => {
    if (!state) return;
    accept(await physicalCall({action:'apply', expected_revision:state.setup.revision, screens:selected}));
    await refreshLayout(); setNotice(tr('输入源已确认，物理屏幕布局已应用'));
  });
  const active = !!state?.active.length;
  const dirty = !!state && JSON.stringify(draft) !== JSON.stringify(state.setup.monitors);
  const scenes = monitorScenes(state?.setup.monitors ?? []);
  const unavailable = !state?.controller_available;
  return <div className="physical" aria-busy={busy}>
    <p>{tr('按实际摆放顺序绑定物理屏幕，再为每块屏幕选择显示哪台电脑。')}</p>
    <p className="setp__note">{tr('首次设置：先让两台电脑都识别所有屏幕，点击识别屏幕，逐一匹配同一块物理屏幕在各电脑中的身份。')}</p>
    {state && <p role="status">{tr('Mac 需要 BetterDisplay；Windows 使用系统 DDC。两端均需安装此修改版并分别保存绑定。')}</p>}
    {error && <p role="alert" className="physical__error">{error}</p>}
    {notice && <p role="status">{notice}</p>}
    {state && <>
      <Group className="physical__actions">
        <Button size="sm" disabled={busy} onClick={() => { void run(async () => { await api.identifyScreens(); }); }}>{tr('识别屏幕')}</Button>
        <Button size="sm" disabled={busy} onClick={() => { void run(async () => { accept(await physicalCall({action:'query'})); await refreshLayout(); }); }}>{tr('刷新')}</Button>
        {active && <Button size="sm" disabled={busy} onClick={() => { void run(async () => { accept(await physicalCall({action:'clear'})); await refreshLayout(); }); }}>{tr('返回普通布局')}</Button>}
      </Group>
      {scenes.length > 0 && <div className="physical__scenes" aria-label={tr('物理屏幕组合')}>
        {scenes.map((scene, index) => <Button key={index} size="sm" variant={scene.length === state.active.length && scene.every((s, i) => sameScreen(s, state.active[i])) ? 'accent' : undefined}
          disabled={busy || dirty || unavailable} onClick={() => { void apply(scene); }}>
          {scene.map(name).join(' / ')}
        </Button>)}
      </div>}
      {active && <p className="setp__note">{tr('当前按物理屏幕归属路由鼠标。修改绑定前，请先返回普通布局。')}</p>}
      <fieldset disabled={busy || active || unavailable} className="physical__setup">
        {draft.map((monitor, index) => <div className="physical__monitor" key={index}>
          <label>{tr('物理屏幕 {0}（从左到右）', index + 1)}
            <Input aria-label={tr('物理屏幕 {0}名称', index+1)} value={monitor.name} onChange={value => update(index, {name:value})}/>
          </label>
          <label>{tr('此电脑上对应的屏幕')}
            <Select aria-label={tr('物理屏幕 {0}控制地址', index+1)} value={monitor.controller_display} options={[{value:'',label:tr('选择屏幕')}, ...local.map(s => ({value:s.key.display,label:`${s.display.name} · ${s.key.display.slice(0,8)}`}))]}
              onChange={value => {
                const key = local.find(s => s.key.display === value)?.key;
                update(index, {controller_display:value, bindings:key ? [...monitor.bindings.filter(b => b.screen.device !== key.device),{screen:key,input:0}] : monitor.bindings});
              }}/>
          </label>
          {computers.map(([device, computer]) => {
            const binding = monitor.bindings.find(b => b.screen.device === device);
            const options = screens.filter(s => s.key.device === device && !s.display.is_virtual);
            const inputKey = `${index}:${device}`;
            const setBinding = (display: string, input: number, raw?: string) => {
              setInputTexts(current => ({...current,[inputKey]:raw ?? (input ? String(input) : '')}));
              update(index, {bindings:[...monitor.bindings.filter(b => b.screen.device !== device), ...(display ? [{screen:{device,display},input}] : [])]});
            };
            return <div key={device} className="physical__binding">
              <label>{computer}
                <Select aria-label={`${monitor.name} · ${computer}`} value={binding?.screen.display ?? ''}
                  options={[{value:'',label:tr('选择屏幕')}, ...options.map(s => ({value:s.key.display,label:`${s.display.name} · ${s.key.display.slice(0,8)}`}))]}
                  onChange={value => setBinding(value, binding?.input ?? 0)}/>
              </label>
              <label>{tr('输入源数值')}
                <Input aria-label={`${monitor.name} · ${computer} · ${tr('输入源数值')}`} value={inputTexts[inputKey] ?? (binding?.input ? String(binding.input) : '')} placeholder="17 / 0x11" inputMode="text" disabled={!binding}
                  onChange={value => { const input = Number(value); setBinding(binding!.screen.display, Number.isInteger(input) && input > 0 && input <= 65535 ? input : 0, value); }}/>
              </label>
              <Group className="physical__actions">
                <Button size="sm" disabled={!binding || !monitor.controller_display} onClick={() => { void run(async () => {
                  const value = await physicalCall<{input:number}>({action:'read',controller_display:monitor.controller_display});
                  setBinding(binding!.screen.display,value.input); setNotice(tr('已读取当前输入源：{0}',value.input));
                }); }}>{tr('读取当前输入')}</Button>
                <Button size="sm" disabled={!binding?.input || !monitor.controller_display} onClick={() => { void run(async () => {
                  setNotice(tr('正在测试，5 秒后自动恢复原输入源'));
                  await physicalCall({action:'test',controller_display:monitor.controller_display,input:binding!.input});
                  setNotice(tr('测试完成，已恢复原输入源；请确认刚才显示的是所选电脑。'));
                }); }}>{tr('测试并恢复')}</Button>
              </Group>
            </div>;
          })}
          <Button size="sm" onClick={() => { setInputTexts({}); setDraft(current => current.filter((_,i) => i !== index)); }}>{tr('移除此物理屏幕')}</Button>
        </div>)}
        <Group className="physical__actions">
          <Button size="sm" disabled={draft.length >= 8} onClick={() => setDraft(current => [...current,{name:tr('物理屏幕 {0}',current.length+1),controller_display:'',bindings:[]}])}>{tr('添加物理屏幕')}</Button>
          <Button size="sm" variant="accent" disabled={!dirty} onClick={() => { void run(async () => { accept(await physicalCall({action:'save',expected_revision:state.setup.revision,monitors:draft})); setNotice(tr('物理屏幕绑定已保存')); }); }}>{tr('保存绑定')}</Button>
        </Group>
      </fieldset>
      <p className="setp__note">{tr('输入源数值由显示器决定。可手动切到目标电脑后读取，或填写已知数值；测试会自动恢复。不会自动扫描未知输入源。')}</p>
      <p className="setp__note">{tr('此功能保留系统扩展桌面，窗口仍可能打开在当前不可见的逻辑屏幕上。')}</p>
    </>}
  </div>;
}
