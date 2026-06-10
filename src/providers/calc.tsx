import { registerProvider, isCopyKey } from './registry';

registerProvider({
  kinds: [],
  Preview: null,

  handleKeyDown: (e, result) => {
    if (isCopyKey(e) && result?.kind === 'calc') {
      e.preventDefault();
      navigator.clipboard.writeText(result.title);
      return true;
    }
    return false;
  },
});
