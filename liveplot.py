"""

Demonstrates the use of multiple Progress instances in a single Live display.    

"""

import sys
import json
from time import sleep

from rich.live import Live
from rich.panel import Panel
from rich.progress import Progress, SpinnerColumn, BarColumn, TextColumn
from rich.table import Table

job_progress=None

progress_table = Table.grid()

with Live(progress_table, refresh_per_second=10):
    for line in sys.stdin:
        line=line[:-1]
        if job_progress is None:
            job_progress = Progress("{task.description}",BarColumn(),TextColumn("[progress.percentage]{task.percentage:>3.0f}%"),
            )
            for ch in json.loads(line): job_progress.add_task(ch,total=1.0)
            progress_table.add_row(Panel.fit(job_progress, title="[b]VU Meter", border_style="red", padding=(1, 2)))
            continue
        vv=[float(v) for v in line.split()]
        for job,val in zip(job_progress.tasks,vv):
            job_progress.update(job.id,completed=val)
