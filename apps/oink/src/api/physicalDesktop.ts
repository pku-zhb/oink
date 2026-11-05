import { call } from './client';
import type { ScreenKey } from './types';
export type DesktopMode = 'complete' | 'compatible';
export interface Endpoint { screen: ScreenKey; input: number; port: string }
export interface Panel { id: string; name: string; width: number; height: number; endpoints: Endpoint[]; readback: boolean; baseline: string; calibrated: boolean }
export interface Mapping { revision: number; group: string; coordinator: string; panels: Panel[] }
export interface Operation { id: string; phase: string; detail: string; deadline_ms: number | null; choices: ScreenKey[]; mode: DesktopMode | null; hardware_verified: boolean }
export interface DesktopState {
  mapping: Mapping | null; imported: Mapping | null; active: ScreenKey[]; mode: DesktopMode | null;
  operation: Operation | null; has_entry: boolean; group: string; cooldown_until: number;
  baseline_required?: boolean; recovery: { Ok: null } | { Err: string } | null; coordinator_error?: string;
}
export interface NativeDisplay {
  key: string; name: string; bounds: { x: number; y: number; w: number; h: number };
  pixel_width: number; pixel_height: number; rotation_deg: number;
  reference_extent?: { w: number; h: number } | null; is_primary: boolean; is_builtin: boolean; is_virtual: boolean;
}
export interface Inspection { device: string; displays: NativeDisplay[]; desktop_supported: boolean; ddc_supported: boolean; desktop_verified: boolean }
export type DesktopRequest =
  | { action: 'status' | 'restore' | 'recover_local' }
  | { action: 'identify'; device: string; display: string; label: string }
  | { action: 'inspect'; device: string }
  | { action: 'read'; device: string; display: string }
  | { action: 'save'; expected_revision: number; mapping: Mapping }
  | { action: 'calibrate'; panel: Panel; target: string }
  | { action: 'apply'; expected_revision: number; choices: ScreenKey[]; mode: DesktopMode; baseline_confirmed: boolean }
  | { action: 'keep' | 'revert'; operation: string };
// Recovery of a calibration that failed before any setup was saved.
export type CalibrationRecovery = { action: 'recover_calibration'; operation: string };
export async function physicalDesktop<R = DesktopState>(request: DesktopRequest | CalibrationRecovery): Promise<R> {
  const result = await call<R & { error?: string }>('physical_monitors', { command: { action: 'experience', request } });
  if (result?.error) throw new Error(result.error);
  if (['status','save','apply','keep','revert','restore','recover_local','recover_calibration','calibrate'].includes(request.action)) {
    const state=result as unknown as DesktopState;
    if (!state || typeof state.group!=='string' || !Array.isArray(state.active) || (state.mapping&&!Array.isArray(state.mapping.panels))) throw new Error('This backend needs the shared monitor workflow update');
  }
  return result;
}
export const busyOperation = (operation: Operation | null) => !!operation && ['checking','preparing','connecting','switching','desktop','awaiting_confirmation','restoring','calibrating'].includes(operation.phase);
export const sameChoice = (a: ScreenKey, b: ScreenKey) => a.device === b.device && a.display === b.display;
/** Presets are optional. Per-monitor selectors remain available at every size. */
export function presets(panels: Panel[]): ScreenKey[][] {
  if (!panels.length) return [];
  let result: ScreenKey[][] = [[]];
  for (const panel of panels) {
    if (!panel.endpoints.length || result.length * panel.endpoints.length > 16) return [];
    result = result.flatMap(row => panel.endpoints.map(endpoint => [...row, endpoint.screen]));
  }
  return result;
}
/** A unique name match is a candidate; calibration supplies confirmation. */
export function suggestPanels(local: Inspection, others: Inspection[]): Panel[] {
  return local.displays.filter(d => !d.is_virtual && !d.is_builtin).map(d => {
    const endpoints: Endpoint[] = [{ screen: {device:local.device,display:d.key}, input:0, port:'' }];
    for (const other of others) {
      const matches = other.displays.filter(s => !s.is_virtual && !s.is_builtin && s.name === d.name && s.pixel_width === d.pixel_width && s.pixel_height === d.pixel_height);
      if (matches.length === 1) endpoints.push({screen:{device:other.device,display:matches[0].key},input:0,port:''});
    }
    return {id:crypto.randomUUID().replaceAll('-',''),name:d.name,width:d.reference_extent?.w ?? d.bounds.w / 1920,height:d.reference_extent?.h ?? d.bounds.h / 1920,endpoints,readback:false,baseline:local.device,calibrated:false};
  });
}
