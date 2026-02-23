{% if read_files %}
<read-files>
{% for f in read_files %}{{ f }}
{% endfor %}</read-files>
{% endif %}
{% if modified_files %}
<modified-files>
{% for f in modified_files %}{{ f }}
{% endfor %}</modified-files>
{% endif %}
