import js from '@eslint/js';
import tseslint from 'typescript-eslint';
import hooks from 'eslint-plugin-react-hooks';
export default tseslint.config(
 {ignores:['build/**','dist/**','node_modules/**']},
 js.configs.recommended,
 ...tseslint.configs.recommended,
 {files:['src/**/*.{ts,tsx}'],plugins:{'react-hooks':hooks},languageOptions:{globals:{window:'readonly',document:'readonly',navigator:'readonly',console:'readonly',localStorage:'readonly',CustomEvent:'readonly',Event:'readonly',requestAnimationFrame:'readonly',setTimeout:'readonly',clearTimeout:'readonly',setInterval:'readonly',clearInterval:'readonly',crypto:'readonly',Blob:'readonly',URL:'readonly',ResizeObserver:'readonly',fetch:'readonly',TextEncoder:'readonly',HTMLInputElement:'readonly',HTMLElement:'readonly',AbortController:'readonly'}},rules:{'react-hooks/rules-of-hooks':'error','react-hooks/exhaustive-deps':'warn','@typescript-eslint/no-explicit-any':'error'}}
);
