// React regression tests read messages in English: their assertions predate `bilingual()` and match
// the English half of what used to be mixed "中文 / English" strings. Only `bilingual()` reads this.
globalThis.document ??= { documentElement: { lang: 'en' } };
