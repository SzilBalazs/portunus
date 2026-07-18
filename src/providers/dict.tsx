import { registerProvider } from './registry';
import DictPreview, { dictCache } from '../components/DictPreview';

registerProvider({
  kinds: ['dict', 'dict-hint'],
  Preview: DictPreview,
  handleLaunch: () => false,

  bindableActions: [
    { id: 'dict:copy', title: 'Copy Definition', hint: 'Dictionary', defaultChord: 'ctrl+c' },
  ],

  actions: (result, ctx) => {
    if (result.kind !== 'dict') return [];
    const copyDefinition = ctx.config?.dict.copy_definition ?? true;
    return [{
      id: 'dict:copy',
      title: copyDefinition ? 'Copy Definition' : 'Copy Word',
      section: 'result',
      shortcut: { ctrl: true, key: 'c' },
      run: () => {
        const cached = dictCache.get(result.title);
        const text = copyDefinition
          ? cached?.definitions[0]?.text ?? result.title
          : result.title;
        navigator.clipboard.writeText(text);
      },
    }];
  },
});
