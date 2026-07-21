import { useState } from 'react'
import { KouCustomApiModal, useToast } from '@kou/ui-kit'
import { useApp } from '../../state/store'
import { api } from '../../lib/api'

const DEFAULT_STANDARD = 'openai-responses'

function standardConfig(standard: string): {
  protocol_format: string
  supported_endpoints: string[]
} {
  switch (standard) {
    case 'openai-responses':
      return { protocol_format: 'openai-responses', supported_endpoints: ['responses'] }
    case 'openai-completions':
      return { protocol_format: 'openai', supported_endpoints: ['chat'] }
    case 'anthropic-messages':
      return { protocol_format: 'claude', supported_endpoints: ['messages'] }
    default:
      return { protocol_format: 'openai-responses', supported_endpoints: ['responses'] }
  }
}

function hostName(url: string): string {
  try {
    return new URL(url).host || 'custom'
  } catch {
    return 'custom'
  }
}

/** Add an endpoint account under the single Custom API line. */
export function CustomApiModal({
  open,
  providerId,
  onClose,
}: {
  open: boolean
  providerId: string | null
  onClose: () => void
}) {
  const { reload } = useApp()
  const toast = useToast()
  const [url, setUrl] = useState('')
  const [apiKey, setApiKey] = useState('')
  const [standard, setStandard] = useState(DEFAULT_STANDARD)
  const [name, setName] = useState('')

  const reset = () => {
    setUrl('')
    setApiKey('')
    setStandard(DEFAULT_STANDARD)
    setName('')
  }

  const close = () => {
    onClose()
    reset()
  }

  const go = async () => {
    if (!providerId) return
    const base = url.trim().replace(/\/+$/, '')
    if (!base) {
      toast('base URL required', 'warn')
      return
    }
    try {
      // eslint-disable-next-line no-new
      new URL(base)
    } catch {
      toast('invalid URL', 'warn')
      return
    }

    const cfg = standardConfig(standard)
    const body = {
      provider_connection_id: providerId,
      label: name.trim() || hostName(base),
      auth_mode: 'api_key',
      api_key: apiKey.trim() || null,
      base_url: base,
      protocol_format: cfg.protocol_format,
      supported_endpoints: cfg.supported_endpoints,
    }

    try {
      await api('/api/provider-accounts', { method: 'POST', body })
      close()
      toast('custom API account connected ✓')
      await reload()
    } catch (err) {
      toast((err as Error).message, 'err')
    }
  }

  return (
    <KouCustomApiModal
      open={open}
      url={url}
      apiKey={apiKey}
      standard={standard}
      name={name}
      onUrlChange={setUrl}
      onApiKeyChange={setApiKey}
      onStandardChange={setStandard}
      onNameChange={setName}
      onClose={close}
      onSubmit={() => void go()}
    />
  )
}
