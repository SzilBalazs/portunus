import { registerProvider, isCopyKey } from './registry';
import DictPreview, { dictCache } from '../components/DictPreview';

registerProvider({
  kinds: ['dict', 'dict-hint'],
  Preview: DictPreview,
  handleLaunch: () => false,

  handleKeyDown: (e, result, ctx) => {
    if (isCopyKey(e) && result?.kind === 'dict') {
      e.preventDefault();
      const copyDefinition = ctx.config?.dict.copy_definition ?? true;
      const cached = dictCache.get(result.title);
      const text = copyDefinition
        ? cached?.definitions[0]?.text ?? result.title
        : result.title;
      navigator.clipboard.writeText(text);
      return true;
    }
    return false;
  },
});
