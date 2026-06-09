import { useCallback, useEffect, useRef, useState } from 'react';
import { Modal } from './ui';
import { useTranslation } from '../i18n';
import { KEYCHAIN_NOTICE_EVENT } from '../utils/keychainNotice';

export default function KeychainNoticeModal() {
  const { t } = useTranslation();
  const [open, setOpen] = useState(false);
  const resolverRef = useRef<((confirmed: boolean) => void) | null>(null);

  const resolveNotice = useCallback((confirmed: boolean) => {
    setOpen(false);
    resolverRef.current?.(confirmed);
    resolverRef.current = null;
  }, []);

  useEffect(() => {
    const handleNotice = (event: Event) => {
      const resolve = (event as CustomEvent<{ resolve?: (confirmed: boolean) => void }>).detail?.resolve;
      if (!resolve) return;
      if (resolverRef.current) {
        resolve(false);
        return;
      }
      resolverRef.current = resolve;
      setOpen(true);
    };

    window.addEventListener(KEYCHAIN_NOTICE_EVENT, handleNotice);
    return () => window.removeEventListener(KEYCHAIN_NOTICE_EVENT, handleNotice);
  }, []);

  return (
    <Modal
      title={t('keychainNotice.title')}
      open={open}
      onOk={() => resolveNotice(true)}
      onCancel={() => resolveNotice(false)}
      okText={t('keychainNotice.continue')}
      cancelText={t('common.cancel')}
      width={480}
      className="keychain-notice-modal"
    >
      <div className="keychain-notice">
        <p>{t('keychainNotice.body')}</p>
        <ul>
          <li>{t('keychainNotice.store')}</li>
          <li>{t('keychainNotice.read')}</li>
          <li>{t('keychainNotice.multiple')}</li>
        </ul>
      </div>
    </Modal>
  );
}
