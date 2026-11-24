import { pinyin } from 'pinyin-pro';

const han = /\p{Script=Han}/u;
const readingCache = new Map();
const surnameCache = new Map();
const cacheLimit = 4096;

function phoneticKeys(text, surname = false) {
  if (!han.test(text)) return [];
  const cache = surname ? surnameCache : readingCache;
  if (cache.has(text)) return cache.get(text);
  const options = { toneType: 'none', type: 'array', surname: surname ? 'head' : 'off' };
  const key = pattern => pinyin(text, { ...options, pattern }).join('')
    .toLowerCase().replaceAll('ü', 'v').replace(/[^a-z0-9]/g, '');
  const keys = [key('pinyin'), key('first')];
  if (cache.size >= cacheLimit) cache.delete(cache.keys().next().value);
  cache.set(text, keys);
  return keys;
}

export function createTextMatcher(query) {
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean).map(text => ({
    text,
    phonetic: /^[a-z0-9ü]+$/.test(text) && /[a-zü]/.test(text) ? text.replaceAll('ü', 'v') : '',
  }));
  if (!terms.length) return () => true;

  return (value, driverNames = []) => {
    const text = String(value ?? '').toLowerCase();
    return terms.every(term => text.includes(term.text) || (term.phonetic && (
      phoneticKeys(text).some(key => key.includes(term.phonetic)) ||
      driverNames.some(name => phoneticKeys(name, true).some(key => key.includes(term.phonetic)))
    )));
  };
}
