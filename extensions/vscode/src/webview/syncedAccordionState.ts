import * as React from "react"

interface SyncedAccordionState {
  signature: string
  value: string[]
}

export function useSyncedAccordionValue(defaultValue: string[]): [string[], (value: string[]) => void] {
  const signature = JSON.stringify(defaultValue)
  const [state, setState] = React.useState<SyncedAccordionState>(() => ({
    signature,
    value: defaultValue
  }))
  const value = state.signature === signature ? state.value : defaultValue

  React.useEffect(() => {
    setState((current) => current.signature === signature ? current : { signature, value: defaultValue })
  }, [signature])

  const setValue = React.useCallback((nextValue: string[]) => {
    setState({ signature, value: nextValue })
  }, [signature])

  return [value, setValue]
}
