import { useState, useEffect } from 'react';
import { Modal, Select, Button, Table, Tag, Space, message, Input, Card, Empty, Spin } from './ui';
import { CloudServerOutlined } from './ui/icons';
import { invoke } from '@tauri-apps/api/core';
import { useTranslation } from '../i18n';

interface CloudInstance {
  instance_id: string;
  name: string;
  ip: string;
  inner_ip: string;
  os: string;
  status: string;
  region: string;
  instance_type: string;
}

interface CloudProvider {
  provider: string;
  name: string;
  access_key_id: string;
  access_key_secret: string;
  regions: string[];
}

const PROVIDERS = [
  { value: 'aliyun', label: '阿里云', defaultRegion: 'cn-hangzhou', regions: ['cn-hangzhou', 'cn-shanghai', 'cn-beijing', 'cn-shenzhen', 'cn-chengdu'] },
  { value: 'aws', label: 'AWS', defaultRegion: 'us-east-1', regions: ['us-east-1', 'us-west-2', 'ap-southeast-1', 'ap-northeast-1', 'eu-west-1'] },
  { value: 'tencent', label: '腾讯云', defaultRegion: 'ap-guangzhou', regions: ['ap-guangzhou', 'ap-shanghai', 'ap-beijing', 'ap-chengdu', 'ap-hongkong'] },
];

export default function CloudImport({ open, onClose, onImported }: { open: boolean; onClose: () => void; onImported: () => void }) {
  const { t, tText } = useTranslation();
  const getProviderLabel = (provider: string) => {
    if (provider === 'aliyun') return tText('cloudImport.provider.aliyun');
    if (provider === 'tencent') return tText('cloudImport.provider.tencent');
    return 'AWS';
  };
  const [providers, setProviders] = useState<CloudProvider[]>([]);
  const [selectedProvider, setSelectedProvider] = useState<string | null>(null);
  const [selectedRegion, setSelectedRegion] = useState<string>('');
  const [instances, setInstances] = useState<CloudInstance[]>([]);
  const [selectedIds, setSelectedIds] = useState<string[]>([]);
  const [fetching, setFetching] = useState(false);
  const [configModalOpen, setConfigModalOpen] = useState(false);
  const [configForm, setConfigForm] = useState({ provider: 'aliyun', access_key_id: '', access_key_secret: '' });

  useEffect(() => {
    if (open) loadProviders();
  }, [open]);

  const loadProviders = async () => {
    try {
      const list = await invoke<CloudProvider[]>('list_cloud_providers');
      setProviders(list);
    } catch { setProviders([]); }
  };

  const handleFetch = async () => {
    if (!selectedProvider || !selectedRegion) {
      message.warning(tText('cloudImport.selectProviderAndRegion'));
      return;
    }
    setFetching(true);
    setInstances([]);
    try {
      const list = await invoke<CloudInstance[]>('fetch_cloud_instances', {
        provider: selectedProvider,
        region: selectedRegion,
      });
      setInstances(list);
      if (list.length === 0) {
        message.info(tText('cloudImport.noInstances'));
      } else {
        message.success(tText('cloudImport.foundInstances', { count: list.length }));
      }
    } catch (e: unknown) {
      message.error(tText('cloudImport.fetchFailed', { error: String(e) }));
    } finally {
      setFetching(false);
    }
  };

  const handleImport = async () => {
    if (selectedIds.length === 0) {
      message.warning(tText('cloudImport.selectInstances'));
      return;
    }
    try {
      const selected = instances.filter((i) => selectedIds.includes(i.instance_id));
      const count = await invoke<number>('import_cloud_instances', { instances: selected });
      message.success(tText('cloudImport.importSuccess', { count }));
      onImported();
      onClose();
    } catch (e: unknown) {
      message.error(tText('cloudImport.importFailed', { error: String(e) }));
    }
  };

  const handleSaveConfig = async () => {
    if (!configForm.access_key_id || !configForm.access_key_secret) {
      message.warning(tText('cloudImport.fillAccessKey'));
      return;
    }
    const prov = PROVIDERS.find((p) => p.value === configForm.provider)!;
    const newProvider: CloudProvider = {
      provider: configForm.provider,
      name: getProviderLabel(configForm.provider),
      access_key_id: configForm.access_key_id,
      access_key_secret: configForm.access_key_secret,
      regions: prov.regions,
    };
    const updated = [...providers.filter((p) => p.provider !== configForm.provider), newProvider];
    try {
      await invoke('save_cloud_providers', { providers: updated });
      message.success(tText('cloudImport.credentialSaved'));
      setProviders(updated);
      setConfigModalOpen(false);
    } catch (e: unknown) {
      message.error(`${e}`);
    }
  };

  const providerInfo = PROVIDERS.find((p) => p.value === selectedProvider);
  const configuredProvider = providers.find((p) => p.provider === selectedProvider);

  return (
    <>
      <Modal
        title={<><CloudServerOutlined /> {t('cloudImport.title')}</>}
        open={open}
        onCancel={onClose}
        width={900}
        footer={
          <Space>
            <Button onClick={onClose}>{t('common.cancel')}</Button>
            <Button type="primary" onClick={handleImport} disabled={selectedIds.length === 0}>
              {t('cloudImport.importCount', { count: selectedIds.length })}
            </Button>
          </Space>
        }
      >
        <Card size="small" style={{ marginBottom: 12 }}>
          <Space wrap style={{ width: '100%', justifyContent: 'space-between' }}>
            <Space>
              <Select style={{ width: 140 }} placeholder={tText('cloudImport.selectProvider')} value={selectedProvider}
                onChange={(v) => {
                  const provider = Array.isArray(v) ? v[0] : v;
                  setSelectedProvider(provider);
                  const p = PROVIDERS.find((x) => x.value === provider);
                  setSelectedRegion(p?.defaultRegion || '');
                }}
                options={PROVIDERS.map((p) => ({ value: p.value, label: getProviderLabel(p.value) }))}
              />
              <Select style={{ width: 160 }} placeholder={tText('cloudImport.selectRegion')} value={selectedRegion}
                onChange={(value) => setSelectedRegion(Array.isArray(value) ? value[0] : value)}
                options={providerInfo?.regions.map((r) => ({ value: r, label: r })) || []}
              />
              <Button type="primary" icon={<CloudServerOutlined />} onClick={handleFetch}
                loading={fetching} disabled={!selectedProvider || !selectedRegion}>
                {t('cloudImport.fetchInstances')}
              </Button>
            </Space>
            <Button onClick={() => {
              if (selectedProvider) {
                const existing = providers.find((p) => p.provider === selectedProvider);
                setConfigForm({
                  provider: selectedProvider,
                  access_key_id: existing?.access_key_id || '',
                  access_key_secret: existing?.access_key_secret || '',
                });
              }
              setConfigModalOpen(true);
            }}>
              {t('cloudImport.configureCredentials')}
            </Button>
          </Space>
          {!configuredProvider && selectedProvider && (
            <div style={{ marginTop: 8, color: '#faad14', fontSize: 12 }}>
              {t('cloudImport.configureProviderFirst', { name: getProviderLabel(selectedProvider) })}
            </div>
          )}
        </Card>

        <Table
          rowKey="instance_id"
          size="small"
          dataSource={instances}
          rowSelection={{
            selectedRowKeys: selectedIds,
            onChange: (keys) => setSelectedIds(keys as string[]),
          }}
          pagination={false}
          scroll={{ y: 350 }}
          columns={[
            { title: t('cloudImport.instanceId'), dataIndex: 'instance_id', width: 160, ellipsis: true },
            { title: t('cloudImport.instanceName'), dataIndex: 'name', width: 140, ellipsis: true },
            { title: t('cloudImport.publicIp'), dataIndex: 'ip', width: 140 },
            { title: t('cloudImport.innerIp'), dataIndex: 'inner_ip', width: 140 },
            { title: t('cloudImport.instanceType'), dataIndex: 'instance_type', width: 100 },
            {
              title: t('cloudImport.instanceStatus'), dataIndex: 'status', width: 80,
              render: (s: string) => <Tag color={s === 'Running' || s === 'running' ? 'green' : 'default'}>{s}</Tag>,
            },
          ]}
          locale={{ emptyText: fetching ? <Spin /> : <Empty description={t('cloudImport.clickToFetch')} image={Empty.PRESENTED_IMAGE_SIMPLE} /> }}
        />
      </Modal>

      {/* 凭据配置 */}
      <Modal title={t('cloudImport.configTitle')} open={configModalOpen} onOk={handleSaveConfig}
        onCancel={() => setConfigModalOpen(false)} destroyOnHidden width={450}>
        <div style={{ marginTop: 16 }}>
          <div style={{ marginBottom: 12 }}>
            <Select style={{ width: '100%' }} value={configForm.provider}
              onChange={(v) => setConfigForm({ ...configForm, provider: Array.isArray(v) ? v[0] : v })}
              options={PROVIDERS.map((p) => ({ value: p.value, label: getProviderLabel(p.value) }))} />
          </div>
          <Input.Password placeholder="Access Key ID" value={configForm.access_key_id}
            onChange={(e) => setConfigForm({ ...configForm, access_key_id: e.target.value })}
            style={{ marginBottom: 12 }} />
          <Input.Password placeholder="Access Key Secret" value={configForm.access_key_secret}
            onChange={(e) => setConfigForm({ ...configForm, access_key_secret: e.target.value })} />
          <div style={{ marginTop: 8, fontSize: 12, color: '#999' }}>
            {t('cloudImport.credentialHint')}
          </div>
        </div>
      </Modal>
    </>
  );
}
