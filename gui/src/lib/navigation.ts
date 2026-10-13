import type {AppState, Route} from './types';

export const navigation = [
  {id:'project', zh:'项目', en:'Project', route:'project'},
  {id:'simulation', zh:'模拟', en:'Simulation', route:'case', tab:'settings'},
  {id:'results', zh:'结果', en:'Results', route:'case', tab:'results'},
  {id:'research', zh:'研究', en:'Research', route:'research'},
  {id:'history', zh:'历史记录', en:'History', route:'history'},
  {id:'resources', zh:'计算资源', en:'Compute resources', route:'resources'},
  {id:'settings', zh:'设置', en:'Settings', route:'settings'},
] as const satisfies readonly {id:string;zh:string;en:string;route:Route;tab?:AppState['caseTab']}[];

export function activeNavigation(state: Pick<AppState, 'route'|'caseTab'>) {
  if (state.route==='case') return state.caseTab==='results'?'results':'simulation';
  if (state.route==='new-case') return 'simulation';
  if (['preparation','versions'].includes(state.route)) return 'project';
  if (state.route==='help') return 'settings';
  return state.route;
}
