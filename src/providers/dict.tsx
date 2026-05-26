import { registerProvider } from './registry';
import DictPreview, { dictCache } from '../components/DictPreview';

registerProvider({
  kinds: ['dict', 'dict-hint'],
  Preview: DictPreview,
  handleLaunch: () => false,

  handleKeyDown: (e, result) => {
    if (e.ctrlKey && !e.altKey && e.key === 'c' && result?.kind === 'dict') {
      e.preventDefault();
      const cached = dictCache.get(result.title);
      navigator.clipboard.writeText(cached?.definitions[0]?.text ?? result.title);
      return true;
    }
    return false;
  },
});
