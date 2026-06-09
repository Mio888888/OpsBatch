export const KEYCHAIN_NOTICE_EVENT = 'opsbatch-keychain-notice';

export function requestKeychainNotice(): Promise<boolean> {
  return new Promise<boolean>((resolve) => {
    window.dispatchEvent(new CustomEvent(KEYCHAIN_NOTICE_EVENT, { detail: { resolve } }));
  });
}
