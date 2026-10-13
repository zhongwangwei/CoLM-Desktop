const paths = {
  project:'M3 7h6l2 2h10v11H3z M3 7V4h6l2 3',
  simulation:'M2 12h4l3-8 4 16 3-8h6',
  results:'M5 20V12h3v8 M11 20V4h3v16 M17 20V8h3v12 M3 20h19',
  research:'M9 3h6 M10 3v7l-5 9a1 1 0 001 2h12a1 1 0 001-2l-5-9V3 M8 15h8',
  history:'M21 12a9 9 0 11-18 0 9 9 0 0118 0 M12 7v5l3 2',
  resources:'M4 3h16v7H4z M4 14h16v7H4z M7 6h1 M7 17h1',
  settings:'M9 3h6l1 3 3 1 2 5-2 5-3 1-1 3H9l-1-3-3-1-2-5 2-5 3-1z M15 12a3 3 0 11-6 0 3 3 0 016 0',
};
export default function NavIcon({name}:{name:keyof typeof paths}) {
  return <svg className="nav-icon" viewBox="0 0 24 24" aria-hidden="true" fill="none" stroke="currentColor" strokeWidth="1.8" strokeLinecap="round" strokeLinejoin="round"><path d={paths[name]}/></svg>;
}
