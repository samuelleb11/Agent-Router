{{/*
Names and labels.
*/}}
{{- define "agent-router.name" -}}
{{- default .Chart.Name .Values.nameOverride | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "agent-router.fullname" -}}
{{- if .Values.fullnameOverride }}
{{- .Values.fullnameOverride | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- $name := default .Chart.Name .Values.nameOverride }}
{{- if contains $name .Release.Name }}
{{- .Release.Name | trunc 63 | trimSuffix "-" }}
{{- else }}
{{- printf "%s-%s" .Release.Name $name | trunc 63 | trimSuffix "-" }}
{{- end }}
{{- end }}
{{- end }}

{{- define "agent-router.chart" -}}
{{- printf "%s-%s" .Chart.Name .Chart.Version | replace "+" "_" | trunc 63 | trimSuffix "-" }}
{{- end }}

{{- define "agent-router.selectorLabels" -}}
app.kubernetes.io/name: {{ include "agent-router.name" . }}
app.kubernetes.io/instance: {{ .Release.Name }}
{{- end }}

{{- define "agent-router.labels" -}}
helm.sh/chart: {{ include "agent-router.chart" . }}
{{ include "agent-router.selectorLabels" . }}
app.kubernetes.io/version: {{ .Chart.AppVersion | quote }}
app.kubernetes.io/managed-by: {{ .Release.Service }}
{{- end }}

{{- define "agent-router.image" -}}
{{- printf "%s:%s" .Values.image.repository (default .Chart.AppVersion .Values.image.tag) }}
{{- end }}

{{/*
The configuration is checked here, once, and a bad one fails the render with a sentence.
A chart that renders a Deployment which then crash-loops says the same thing an hour
later and less clearly.
*/}}
{{- define "agent-router.validate" -}}
{{- if and .Values.ingress.enabled (not .Values.ingress.host) }}
{{- fail "ingress.enabled is true but ingress.host is empty" }}
{{- end }}
{{- /* Counted in characters, as the router counts them: `len` would count bytes. */}}
{{- if and .Values.auth.password (lt (len (splitList "" .Values.auth.password)) 8) }}
{{- fail "auth.password is shorter than 8 characters, and the router refuses to start with it" }}
{{- end }}
{{- /*
  On SIGTERM the router keeps accepting for stopDelaySeconds, waits up to drainSeconds for
  open requests, then gives its capture 5 s more. The kubelet kills the pod when the grace
  period ends, and whatever capture has not written by then is lost.
*/}}
{{- $stop := int64 (.Values.server.stopDelaySeconds | default 0) }}
{{- $drain := int64 .Values.server.drainSeconds }}
{{- $grace := int64 .Values.terminationGracePeriodSeconds }}
{{- if ge (add $stop $drain 5) $grace }}
{{- fail (printf "server.stopDelaySeconds (%d) and server.drainSeconds (%d), and then 5 s for the router's capture: all of it must end before terminationGracePeriodSeconds (%d), when the kubelet kills the pod. Lower one of them or raise terminationGracePeriodSeconds" $stop $drain $grace) }}
{{- end }}
{{- if not (has .Values.rollout.strategy (list "RollingUpdate" "Recreate")) }}
{{- fail (printf "rollout.strategy is %q: it is RollingUpdate or Recreate" .Values.rollout.strategy) }}
{{- end }}
{{- end }}

{{/*
Every name the router answers to besides IP literals and localhost: the ingress host, the
Service's own names (so Northstar, or anything else in the cluster, can call it by name),
then any listed. Lowercase and without repeats, as the router compares them; the router adds
the public URL's host itself.
*/}}
{{- define "agent-router.allowHosts" -}}
{{- $svc := include "agent-router.fullname" . }}
{{- $ns := .Release.Namespace }}
{{- $hosts := list }}
{{- if .Values.ingress.enabled }}
{{- $hosts = append $hosts .Values.ingress.host }}
{{- end }}
{{- $hosts = concat $hosts (list $svc (printf "%s.%s" $svc $ns) (printf "%s.%s.svc" $svc $ns) (printf "%s.%s.svc.cluster.local" $svc $ns)) }}
{{- range .Values.server.allowHosts }}
{{- $hosts = append $hosts . }}
{{- end }}
{{- $lower := list }}
{{- range $hosts }}
{{- $lower = append $lower (lower .) }}
{{- end }}
{{- $lower | uniq | toJson }}
{{- end }}

{{/*
The URL clients use: server.publicUrl, else the ingress's, else nothing -- a port-forward
has no URL worth telling the router.
*/}}
{{- define "agent-router.publicUrl" -}}
{{- if .Values.server.publicUrl }}
{{- .Values.server.publicUrl | trimSuffix "/" }}
{{- else if .Values.ingress.enabled }}
{{- if .Values.ingress.tls.enabled }}https{{ else }}http{{ end }}://{{ .Values.ingress.host }}
{{- end }}
{{- end }}

{{/*
Where the router's password lives (pinned at every start).
*/}}
{{- define "agent-router.auth.secretName" -}}
{{- default (printf "%s-auth" (include "agent-router.fullname" .) | trunc 63 | trimSuffix "-") .Values.auth.existingSecret }}
{{- end }}

{{/*
The data directory's claim: the one this chart makes, or yours.
*/}}
{{- define "agent-router.data.claimName" -}}
{{- default (printf "%s-data" (include "agent-router.fullname" .)) .Values.persistence.existingClaim }}
{{- end }}
