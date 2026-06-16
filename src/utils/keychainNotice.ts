import { invoke } from '@tauri-apps/api/core';

export const KEYCHAIN_NOTICE_EVENT = 'opsbatch-keychain-notice';

let unlockPromise: Promise<boolean> | null = null;

export function requestKeychainNotice(): Promise<boolean> {
  if (!unlockPromise) {
    unlockPromise = new Promise<boolean>((resolve) => {
      invoke<boolean>('is_local_vault_unlocked_command')
        .then((unlocked) => {
          if (unlocked) {
            resolve(true);
            return;
          }
          window.dispatchEvent(new CustomEvent(KEYCHAIN_NOTICE_EVENT, { detail: { resolve } }));
        })
        .catch(() => resolve(false));
    })
      .finally(() => {
        unlockPromise = null;
      });
  }
  return unlockPromise;
}

export async function unlockLocalVault(masterKey?: string | null): Promise<void> {
  await invoke('unlock_local_vault', { masterKey: masterKey ?? null });
}

export async function migratePlaintextSecretsToVault(): Promise<void> {
  await invoke('migrate_plaintext_secrets_to_vault');
}

export async function runStartupRepoUpdates(): Promise<void> {
  await invoke('run_startup_repo_updates');
}
