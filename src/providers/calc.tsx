import { registerProvider } from './registry';

registerProvider({
  kinds: [],
  Preview: null,

  bindableActions: [
    { id: 'calc:copy', title: 'Copy Result', hint: 'Calculator', defaultChord: 'ctrl+c' },
  ],

  actions: result => {
    if (result.kind !== 'calc') return [];
    return [{
      id: 'calc:copy',
      title: 'Copy Result',
      section: 'result',
      shortcut: { ctrl: true, key: 'c' },
      run: () => { navigator.clipboard.writeText(result.title); },
    }];
  },
});
