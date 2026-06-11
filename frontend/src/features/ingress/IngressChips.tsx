import { useToast } from '../../components/ui/toast'
import { Panel, PanelHeader, PanelTitle } from '../../components/ui/Panel'
import { Chip } from '../../components/ui/Chip'

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
  return (
    <Panel>
      <PanelHeader>
        <PanelTitle kana="入口">INGRESS</PanelTitle>
        <span className="mut mono" style={{ marginLeft: 'auto', fontSize: 10, letterSpacing: '.14em' }}>
          click = copy
        </span>
      </PanelHeader>
      <div className="chips" style={{ paddingBottom: 6 }}>
        <Chip mono className="ingress-url" onClick={copy}>{url}</Chip>
      </div>
      <p className="mut" style={{ margin: 0, padding: '0 18px 14px', fontSize: 11.5 }}>
        chat · completions · responses · messages · embeddings · images · audio ·
        moderations · rerank · search — все под одним base URL
      </p>
    </Panel>
  )
}
