import { KouIngressChips, useToast } from '@kou/ui-kit'

/** Ingress: one absolute base URL — every route family (chat, responses,
 *  messages, embeddings, audio, …) lives under /v1, so the URL is all
 *  a client needs. Click to copy. */
export function IngressChips() {
  const toast = useToast()
  const url = location.origin + '/v1'
  const copy = () => {
    navigator.clipboard?.writeText(url)
      .then(() => toast('copied ' + url))
      .catch(() => toast('clipboard unavailable', 'warn'))
  }
  return <KouIngressChips url={url} onCopy={copy} />
}
