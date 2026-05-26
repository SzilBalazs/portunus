import { registerProvider } from './registry';

registerProvider({
  kinds: [],
  Preview: null,

  handleKeyDown: (e, result) => {
    if (e.ctrlKey && !e.altKey && e.key === 'c' && result?.kind === 'calc') {
      e.preventDefault();
      navigator.clipboard.writeText(result.title);
      return true;
    }
    return false;
  },
});
