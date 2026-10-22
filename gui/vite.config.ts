import {defineConfig} from 'vite';
import react from '@vitejs/plugin-react';
import tailwindcss from '@tailwindcss/vite';
import {fileURLToPath, URL} from 'node:url';
export default defineConfig({
 plugins:[react(),tailwindcss()],
 resolve:{alias:{'@':fileURLToPath(new URL('./src',import.meta.url))}},
 base:'./',
 server:{port:18820,strictPort:true},
 // 发布包不带 sourcemap（原来 6.1 MB 产物里 4.3 MB 是 .map，随包装进应用）；排查时 COLM_SOURCEMAP=1 再打开。
 build:{outDir:'build',emptyOutDir:true,target:'es2022',sourcemap:process.env.COLM_SOURCEMAP==='1'},
 publicDir:'public',
});
